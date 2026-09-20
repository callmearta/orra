//! Streaming speech-to-text, from whichever provider is configured.
//!
//! One session loop serves every provider: open the microphone, let it fill
//! memory while the socket opens, feed the socket, and on release drain what
//! the microphone still holds before asking for the final transcript. What
//! actually differs between providers — the URL, how audio is framed, how a
//! server message is read, what closes the stream — is behind [`Wire`], and
//! lives in one module per provider.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::audio::Capture;
use crate::config::{Config, Provider};
use crate::{assemblyai, deepgram, gemini};

/// The events the backend emits. The frontends listen by these names.
pub const EVT_STT: &str = "stt";
pub const EVT_LEVEL: &str = "mic-level";
pub const EVT_STATE: &str = "state";
pub const EVT_LANGUAGE: &str = "language";
pub const EVT_HISTORY: &str = "history";
pub const EVT_ERROR: &str = "error";

/// How long the socket handshake is given before the dictation is abandoned.
/// Generous, because the microphone is already recording: a slow connect costs
/// memory, not words.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the device thread is given to hand over its last samples after the
/// microphone is told to stop. It normally closes the channel within a poll
/// tick or two, so this only ever bounds a device that is slow to close.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// How long the clouds are given to flush once the stream has been asked to
/// close. What a provider that is not a cloud uses is its own `Wire::flush`.
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(4);

/// A running dictation. Dropping it (or calling `stop`) flushes the stream.
pub struct SttSession {
    stop: Option<oneshot::Sender<()>>,
}

impl SttSession {
    /// A session that ends when `stop` is sent.
    ///
    /// For providers that do not go through [`start`]: the local one's HTTP
    /// transports run their own loop, and what the rest of the app holds has to
    /// be the same handle either way.
    pub(crate) fn from_stop(stop: oneshot::Sender<()>) -> Self {
        Self { stop: Some(stop) }
    }

    /// Ask the provider to flush and close; the text is delivered after that.
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
    }
}

/// What one dictation has heard so far.
#[derive(Default)]
pub struct Session {
    /// Finished utterances, joined with a space, in the order they were spoken.
    pub finals: String,
    /// The tail of a sentence still being spoken.
    pub interim: String,
    pub confidence: Confidence,
    /// Set once the provider will take audio. Gemini has to acknowledge its
    /// setup message first; the others are ready the moment the socket is open.
    pub ready: bool,
    /// Set once the user has let go and the close frames have gone out. Some
    /// providers only send the last of the transcript in response to that.
    pub closing: bool,
}

impl Session {
    /// A finished utterance: appended to the transcript.
    pub fn push_final(&mut self, text: &str, confidence: Option<f32>) {
        let text = text.trim();
        if !text.is_empty() {
            if !self.finals.is_empty() {
                self.finals.push(' ');
            }
            self.finals.push_str(text);
            // A provider that does not report confidence simply does not
            // contribute to the mean.
            if let Some(c) = confidence {
                self.confidence.add(c, text.split_whitespace().count());
            }
        }
        self.interim.clear();
    }

    /// The tail of a sentence still being spoken, replacing the last one.
    pub fn set_interim(&mut self, text: &str) {
        self.interim = text.trim().to_string();
    }

    /// Add to the tail of a sentence still being spoken.
    ///
    /// Separate from [`Session::set_interim`] because a provider that sends each
    /// fragment once — the Realtime protocol does — cannot replace the tail
    /// without losing what came before it, and the whitespace at the edges of a
    /// fragment is the only thing saying where one word ends and the next
    /// begins. Only the leading edge of a fresh tail is trimmed, so consecutive
    /// fragments join the way the server wrote them.
    pub fn append_interim(&mut self, delta: &str) {
        if self.interim.is_empty() {
            self.interim = delta.trim_start().to_string();
        } else {
            self.interim.push_str(delta);
        }
    }

    /// What the overlay should be showing: the transcript so far, and the words
    /// that are still being spoken.
    pub fn show(&self, app: &AppHandle) {
        let _ = app.emit(EVT_STT, json!({ "final": self.finals, "interim": self.interim }));
    }
}

/// Running mean of the provider's per-result confidence for one dictation.
///
/// Weighted by word count so a one-word final cannot count as much as a long
/// sentence, and kept separate from the transcript because the transcript is a
/// string and this is not.
#[derive(Default)]
pub struct Confidence {
    weighted: f64,
    weight: f64,
}

impl Confidence {
    fn add(&mut self, value: f32, words: usize) {
        let w = words.max(1) as f64;
        self.weighted += value as f64 * w;
        self.weight += w;
    }

    pub fn mean(&self) -> Option<f32> {
        (self.weight > 0.0).then(|| (self.weighted / self.weight) as f32)
    }
}

/// Whether the session should carry on reading the stream.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Done,
}

/// Where a provider listens, and how to talk to it.
pub struct Endpoint {
    pub url: String,
    /// The header the key goes in, spelled the way this provider wants it, or
    /// `None` when the key travels another way — Gemini carries it in the URL,
    /// and a local server usually has none at all.
    pub auth: Option<(&'static str, String)>,
    pub wire: Wire,
}

/// Everything a provider has to answer for the shared session loop.
pub struct Wire {
    /// Sent as soon as the socket opens, before any audio.
    ///
    /// A function of the settings rather than a constant, because the local
    /// provider has to name its model and its language in there and both of
    /// those are settings; the clouds ignore the argument.
    pub handshake: fn(&Config) -> Vec<String>,
    /// Whether the socket must wait for the provider to answer the handshake
    /// before it will take audio. Gemini does; the others are happy either way.
    pub awaits_handshake: bool,
    /// Sent once every sample has been written, to ask for the last transcript.
    pub close: &'static [&'static str],
    /// How long the provider is given to flush once the stream has been asked
    /// to close.
    ///
    /// Per provider, because what is being waited for is a model decoding the
    /// last utterance: a cloud answers in about as long as the round trip, while
    /// a model on this machine can take tens of seconds over a long dictation
    /// and would be cut off mid-sentence by the cloud's deadline.
    pub flush: Duration,
    /// How many samples to gather into one frame, for providers that only take
    /// audio in sizeable blocks. Zero sends the device's own blocks as they
    /// arrive, which is the lowest latency and what Deepgram wants.
    pub frame_samples: fn(u32) -> usize,
    /// One frame of samples, ready for the socket.
    pub encode: fn(&[i16], u32) -> Message,
    /// One server message, read into the transcript. Deliberately free of the
    /// app handle, so a provider's parser can be tested on its own.
    pub decode: fn(&str, &mut Session) -> Flow,
}

/// Open a stream to the configured provider and start feeding it the microphone.
///
/// Returns before the socket exists. The microphone is already recording at
/// that point — its samples queue in memory until the handshake finishes, so
/// the words spoken while the socket was opening are transcribed along with the
/// rest, and the overlay has something to show from the instant the key goes
/// down rather than a second later.
///
/// The local provider is dispatched before any of that: its transports are not
/// sockets, and its session is built in [`crate::local`]. What comes back is the
/// same handle either way, so nothing above this cares which one it got.
pub fn start(
    app: AppHandle,
    cfg: &Config,
    id: u64,
    capture: Capture,
    purpose: crate::state::Purpose,
) -> Result<SttSession> {
    // A server the user runs is a different shape of session — its audio is
    // gathered and handed over rather than streamed, and there is no key to
    // insist on — so it brings its own loop. Every one of those providers,
    // named or custom or run by this app, is the same code from here.
    if cfg.provider.is_self_hosted() {
        return crate::local::start(app, cfg, id, capture, purpose);
    }

    let key = cfg.api_key().ok_or_else(|| {
        anyhow!(
            "No {} API key. Add {} to .env or paste it in Settings.",
            cfg.provider.label(),
            cfg.provider.env_var()
        )
    })?;

    // Gemini carries its key in the URL; the other two send it as a header,
    // spelled differently by each.
    let endpoint = match cfg.provider {
        Provider::Deepgram => Endpoint {
            url: deepgram::build_url(&deepgram::SttOptions::from_config(cfg), capture.sample_rate),
            auth: Some(("Authorization", format!("Token {key}"))),
            wire: deepgram::WIRE,
        },
        Provider::AssemblyAi => Endpoint {
            url: assemblyai::url(capture.sample_rate, &cfg.language),
            auth: Some(("Authorization", key)),
            wire: assemblyai::WIRE,
        },
        Provider::Gemini => Endpoint { url: gemini::url(&key), auth: None, wire: gemini::WIRE },
        // All returned above, before any key was asked for.
        Provider::Ollama
        | Provider::Speaches
        | Provider::LocalAi
        | Provider::WhisperCpp
        | Provider::Local
        | Provider::Orra => unreachable!("self-hosted sessions are dispatched before this match"),
    };

    Ok(spawn_session(app, cfg.clone(), id, capture, purpose, endpoint))
}

/// Drive one dictation over a socket.
///
/// The clouds arrive here through [`start`]; the local provider's Realtime
/// transport calls it directly, because the loop below is the same work whoever
/// is on the other end — only the URL, the auth header and the [`Wire`] differ.
pub fn spawn_session(
    app: AppHandle,
    cfg: Config,
    id: u64,
    mut capture: Capture,
    purpose: crate::state::Purpose,
    endpoint: Endpoint,
) -> SttSession {
    let Endpoint { url, auth, wire } = endpoint;
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let mic = capture.handle();
        let mut session = Session { ready: !wire.awaits_handshake, ..Default::default() };

        let mut closing = false;
        let mut mic_stopped = false;
        // Monotonic, so a wall-clock jump mid-dictation cannot produce a
        // negative or absurd duration. Read before the connect, which is time
        // the microphone was live and the user may well have been speaking.
        let started = Instant::now();
        // Stamped when the stop arrives rather than when the stream finally
        // closes: the flush wait afterwards is not time spent speaking.
        let mut stopped: Option<Instant> = None;

        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        // Far-future deadlines, reset as each phase of the close begins.
        let connect_deadline = tokio::time::sleep(CONNECT_TIMEOUT);
        let drain = tokio::time::sleep(Duration::from_secs(3600));
        let flush_deadline = tokio::time::sleep(Duration::from_secs(3600));
        tokio::pin!(connect_deadline, drain, flush_deadline);

        let connecting = async {
            let mut request = url
                .as_str()
                .into_client_request()
                .context("building the request")?;
            if let Some((name, value)) = &auth {
                request.headers_mut().insert(
                    *name,
                    HeaderValue::from_str(value).context("API key is not a valid header")?,
                );
            }
            tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| anyhow!("could not reach the transcription service: {e}"))
        };
        tokio::pin!(connecting);

        // Phase one: the microphone fills the channel while the socket opens.
        // Nothing is read from that channel yet, which is the point — the
        // handshake takes most of a second, and the audio recorded during it has
        // to survive to be sent.
        let socket = loop {
            tokio::select! {
                opened = &mut connecting => break opened,
                _ = &mut stop_rx, if !closing => {
                    closing = true;
                    stopped = Some(Instant::now());
                    let _ = app.emit(EVT_STATE, "processing");
                }
                _ = &mut connect_deadline => break Err(anyhow!(
                    "the transcription service did not answer within {}s",
                    CONNECT_TIMEOUT.as_secs()
                )),
                _ = ticker.tick() => {
                    let _ = app.emit(EVT_LEVEL, mic.level());
                }
            }

            if closing && !mic_stopped {
                mic_stopped = true;
                mic.shutdown();
            }
        };

        // Releasing the key before the socket opened is not a reason to throw the
        // dictation away — there is nothing to feed it to yet, so wait for the
        // handshake and send what was captured.
        let socket = match socket {
            Ok((socket, _)) => socket,
            Err(e) => {
                crate::problem::report(&app, "Could not start the transcription service", &e);
                crate::state::abandon_dictation(&app, id);
                return;
            }
        };
        let (mut write, mut read) = socket.split();

        for frame in (wire.handshake)(&cfg) {
            let _ = write.send(Message::Text(frame.into())).await;
        }

        // The mic may have been stopped while the handshake was still running,
        // in which case the give-up clock starts here: it measures how long the
        // device thread takes to hand over its samples, not how long the socket
        // took to open. Started any earlier it would expire before the first
        // byte was ever sent, dropping exactly the audio this is here to save.
        if closing {
            drain.as_mut().reset(tokio::time::Instant::now() + DRAIN_TIMEOUT);
        }

        // Phase two: the socket is live, so the queue goes out and the stream
        // runs as normal.
        let mut drained = false;
        let mut flushed = false;
        // Samples waiting to make up a full frame, for providers that take
        // audio in blocks rather than in whatever the device hands over.
        let mut pending: Vec<i16> = Vec::new();
        let per_frame = (wire.frame_samples)(capture.sample_rate);

        loop {
            tokio::select! {
                _ = &mut stop_rx, if !closing => {
                    closing = true;
                    stopped = Some(Instant::now());
                    let _ = app.emit(EVT_STATE, "processing");
                }
                res = capture.rx.recv(), if session.ready && !drained => match res {
                    Some(samples) => {
                        if per_frame == 0 {
                            if write.send((wire.encode)(&samples, capture.sample_rate)).await.is_err() {
                                break;
                            }
                        } else {
                            pending.extend_from_slice(&samples);
                            if pending.len() >= per_frame {
                                let frame = std::mem::take(&mut pending);
                                if write.send((wire.encode)(&frame, capture.sample_rate)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    // The device thread has exited and handed over everything it
                    // captured: there is nothing left to wait for.
                    None => drained = true,
                },
                frame = read.next() => {
                    // Providers answer in JSON, but not always in *text* frames:
                    // Google's Live API sends the same JSON in binary ones.
                    let message = match frame {
                        Some(Ok(Message::Text(t))) => Some(t.to_string()),
                        Some(Ok(Message::Binary(b))) => String::from_utf8(b.to_vec()).ok(),
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(e)) => {
                            crate::problem::report(
                                &app,
                                "The transcription service dropped the connection",
                                format!("stream error: {e}"),
                            );
                            break;
                        }
                        _ => None,
                    };
                    if let Some(raw) = message {
                        if (wire.decode)(&raw, &mut session) == Flow::Done {
                            break;
                        }
                        session.show(&app);
                    }
                }
                // A provider that never acknowledged its handshake would leave
                // this waiting forever with audio piling up behind it.
                _ = &mut connect_deadline, if !session.ready => {
                    crate::problem::report(
                        &app,
                        "The transcription service never answered",
                        // Worded as the other deadline is: `problem::Kind` reads
                        // it to decide the headline, and "no response within 10s"
                        // said nothing it recognised.
                        format!("did not answer within {}s", CONNECT_TIMEOUT.as_secs()),
                    );
                    crate::state::abandon_dictation(&app, id);
                    return;
                }
                _ = &mut flush_deadline, if flushed => break,
                _ = ticker.tick() => {
                    let _ = app.emit(EVT_LEVEL, mic.level());
                }
            }

            if closing && !mic_stopped {
                mic_stopped = true;
                mic.shutdown();
                drain.as_mut().reset(tokio::time::Instant::now() + DRAIN_TIMEOUT);
            }
            // A device thread that never closes its channel would hold the
            // dictation open forever; it only ever needs a poll tick, so a
            // wedged one is given up on instead.
            if closing && !drained && drain.is_elapsed() {
                drained = true;
            }
            // Letting go never cuts audio off: everything the microphone still
            // holds goes out first, and only then is the stream closed.
            if closing && drained && !flushed && session.ready {
                if !pending.is_empty() {
                    // A part-frame is padded with silence rather than dropped,
                    // or sent short: the provider takes blocks and would refuse
                    // a sliver, and silence costs nothing to transcribe.
                    if pending.len() < per_frame {
                        pending.resize(per_frame, 0);
                    }
                    let _ = write.send((wire.encode)(&pending, capture.sample_rate)).await;
                    pending.clear();
                }
                for frame in wire.close {
                    let _ = write.send(Message::Text((*frame).into())).await;
                }
                session.closing = true;
                flushed = true;
                flush_deadline.as_mut().reset(tokio::time::Instant::now() + wire.flush);
            }
        }

        let raw = session.finals.clone();
        let spoken = crate::state::Spoken {
            // A stream that ended on its own never saw a stop, so the elapsed
            // time at this point is the best measure available.
            ms: stopped
                .unwrap_or_else(Instant::now)
                .duration_since(started)
                .as_millis() as u64,
            confidence: session.confidence.mean(),
        };
        let _ = app.emit(EVT_LEVEL, 0.0f32);
        crate::state::finish_dictation(&app, id, &raw, &cfg, spoken, purpose).await;
    });

    SttSession { stop: Some(stop_tx) }
}

/// Percent-encode everything outside the unreserved set (RFC 3986).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_is_weighted_by_word_count() {
        let mut c = Confidence::default();
        assert_eq!(c.mean(), None, "nothing measured yet");

        c.add(0.9, 1); // one word at 0.9
        c.add(0.5, 3); // three words at 0.5
        // (0.9 * 1 + 0.5 * 3) / 4 = 0.6 — the long result dominates, which is
        // the point of weighting: a clipped one-word final must not swing it.
        let mean = c.mean().expect("measured");
        assert!((mean - 0.6).abs() < 1e-6, "got {mean}");
    }

    #[test]
    fn a_wordless_result_still_counts_once() {
        // `max(1)` in `add` keeps a zero-word final from contributing weight 0,
        // which would otherwise let it divide by nothing.
        let mut c = Confidence::default();
        c.add(0.8, 0);
        assert_eq!(c.mean(), Some(0.8));
    }
}
