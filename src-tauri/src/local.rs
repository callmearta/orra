//! Speech-to-text from a server the user runs themselves.
//!
//! Nothing about that server is known in advance — it is a URL and a model name
//! — so this module is written around the three shapes such servers actually
//! speak, and the user says which one theirs is:
//!
//! * [`LocalTransport::Http`] posts the recording when the key is released and
//!   reads the transcript out of the reply. This is the OpenAI
//!   `/audio/transcriptions` shape, which is what Ollama's audio models,
//!   LocalAI, Speaches, faster-whisper-server, vLLM and LM Studio answer.
//! * [`LocalTransport::Sse`] posts the same request asking for `stream=true`,
//!   and reads the transcript as the server decodes it.
//! * [`LocalTransport::WebSocket`] is the OpenAI Realtime socket, which takes
//!   audio as it is spoken and sends partial text back — the only one of the
//!   three that can show words in the overlay while the user is still talking.
//!
//! The transports do not share a session loop: the first two cannot say
//! anything until the recording has ended, so they gather the audio and hand it
//! over, while the third is a socket like every cloud provider and rides the
//! loop in [`crate::stt`].

use std::io::BufRead as _;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::audio::{self, Capture};
use crate::config::{Config, LocalTransport};
use crate::state::Purpose;
use crate::stt::{Flow, Session, SttSession, Wire, EVT_LEVEL, EVT_STATE};

/// How long a transcription is given before the dictation gives up on it.
///
/// Generous for the same reason the translation timeout is: the model may be a
/// cold one on this machine, where loading it is most of the wait, and a request
/// that is still decoding is not the same thing as one that failed.
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

/// The rate the recording is sent at.
///
/// 16 kHz mono is what speech models are trained on, and it is the only rate
/// whisper.cpp's server takes without being started with `--convert`.
const SPEECH_RATE: u32 = 16_000;

/// The rate the Realtime socket is told to expect, and what [`encode`]
/// resamples to. It is declared in the session update, so a server that would
/// rather have something else has been told what is coming rather than left to
/// guess.
const REALTIME_RATE: u32 = 24_000;

/// How long the server is given to finish the last utterance once the audio has
/// been committed. Much longer than a cloud needs — this is a process on this
/// machine decoding a whole dictation — but bounded, because a dictation that
/// never finishes is worse than one that gives up.
const FLUSH: Duration = Duration::from_secs(30);

/// How long the device thread is given to hand over its last samples.
const DRAIN: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// starting a dictation
// ---------------------------------------------------------------------------

/// Begin a dictation against the configured server.
pub fn start(
    app: AppHandle,
    cfg: &Config,
    id: u64,
    capture: Capture,
    purpose: Purpose,
) -> Result<SttSession> {
    // The one setting with no sensible default: there is no port or path this
    // app could guess at, and guessing wrong fails halfway through a dictation
    // rather than here.
    if cfg.local_base_url.trim().is_empty() {
        return Err(anyhow!(
            "No endpoint URL set. Paste one in Settings, under Transcription."
        ));
    }

    if cfg.local_transport == LocalTransport::WebSocket {
        return Ok(crate::stt::spawn_session(
            app,
            cfg.clone(),
            id,
            capture,
            purpose,
            crate::stt::Endpoint {
                url: realtime_url(cfg),
                auth: bearer(&cfg.local_key),
                wire: WIRE,
            },
        ));
    }
    Ok(spawn_post(app, cfg.clone(), id, capture, purpose))
}

/// Record, then hand the whole recording over.
///
/// The shape both HTTP transports need: neither can send anything until the
/// audio exists, which is the moment the key is released.
fn spawn_post(
    app: AppHandle,
    cfg: Config,
    id: u64,
    mut capture: Capture,
    purpose: Purpose,
) -> SttSession {
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let mic = capture.handle();
        let started = Instant::now();
        let mut stopped: Option<Instant> = None;
        let mut closing = false;
        let mut mic_stopped = false;
        let mut drained = false;
        let mut samples: Vec<i16> = Vec::new();

        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        let drain = tokio::time::sleep(Duration::from_secs(3600));
        tokio::pin!(drain);

        loop {
            tokio::select! {
                _ = &mut stop_rx, if !closing => {
                    closing = true;
                    stopped = Some(Instant::now());
                    let _ = app.emit(EVT_STATE, "processing");
                }
                res = capture.rx.recv(), if !drained => match res {
                    Some(chunk) => samples.extend_from_slice(&chunk),
                    // The device thread has exited and handed over everything
                    // it captured: there is nothing left to wait for.
                    None => drained = true,
                },
                _ = &mut drain, if closing && !drained => drained = true,
                _ = ticker.tick() => {
                    let _ = app.emit(EVT_LEVEL, mic.level());
                }
            }

            if closing && !mic_stopped {
                mic_stopped = true;
                mic.shutdown();
                drain.as_mut().reset(tokio::time::Instant::now() + DRAIN);
            }
            // Everything the microphone holds has been read: the recording is
            // whole, and it can go.
            if closing && drained {
                break;
            }
        }

        let _ = app.emit(EVT_LEVEL, 0.0f32);
        let spoken = crate::state::Spoken {
            // A stream that ended without a stop never saw one, so the elapsed
            // time is the best measure available.
            ms: stopped
                .unwrap_or_else(Instant::now)
                .duration_since(started)
                .as_millis() as u64,
            // These servers do not report one, and inventing a number would put
            // it into the Insights averages as though it had been measured.
            confidence: None,
        };

        // A key tapped and released before the microphone opened: nothing was
        // said, so nothing is sent and there is no error to report.
        if samples.is_empty() {
            crate::state::finish_dictation(&app, id, "", &cfg, spoken, purpose).await;
            return;
        }

        // Resampled here rather than left to the server: every one of these
        // accepts 16 kHz, and the ones that take the file as-is would otherwise
        // be handed a rate they were not started for.
        let wav = audio::wav_bytes(
            &audio::resample_to(&samples, capture.sample_rate, SPEECH_RATE),
            SPEECH_RATE,
        );

        let stream = cfg.local_transport == LocalTransport::Sse;
        let overlay = app.clone();
        let settings = cfg.clone();
        // The request blocks, so it runs off the async runtime — and on the
        // streaming transport it reports back into the overlay from there,
        // which is where the words actually arrive.
        let done = tokio::task::spawn_blocking(move || match stream {
            true => transcribe_sse(&settings, &wav, &mut |text| show(&overlay, text)),
            false => transcribe_http(&settings, &wav),
        })
        .await;

        match done {
            Ok(Ok(text)) => {
                show(&app, &text);
                crate::state::finish_dictation(&app, id, &text, &cfg, spoken, purpose).await;
            }
            Ok(Err(e)) => {
                crate::problem::report(&app, "Could not transcribe with the local server", &e);
                crate::state::abandon_dictation(&app, id);
            }
            Err(e) => {
                crate::problem::report(
                    &app,
                    "Could not transcribe with the local server",
                    format!("transcription task failed: {e}"),
                );
                crate::state::abandon_dictation(&app, id);
            }
        }
    });

    SttSession::from_stop(stop_tx)
}

/// Put text in the overlay, as the socket loop in [`crate::stt`] does.
fn show(app: &AppHandle, text: &str) {
    let _ = app.emit(crate::stt::EVT_STT, json!({ "final": text, "interim": "" }));
}

// ---------------------------------------------------------------------------
// the HTTP transports
// ---------------------------------------------------------------------------

/// Send the recording and read the transcript out of the reply.
pub fn transcribe_http(cfg: &Config, wav: &[u8]) -> Result<String> {
    let mut body = post_audio(cfg, wav, false)?;
    let body = body.read_to_vec().map_err(|e| anyhow!("reading the reply: {e}"))?;
    parse_transcript(&body)
}

/// Send the recording and read the transcript as the server decodes it.
///
/// The deltas arrive only after the whole recording has been handed over —
/// nothing can be transcribed before the audio exists — so this shortens the
/// wait for the first word rather than typing while the user is still speaking.
/// Only the WebSocket transport can do that.
fn transcribe_sse(cfg: &Config, wav: &[u8], on_text: &mut dyn FnMut(&str)) -> Result<String> {
    let body = post_audio(cfg, wav, true)?;
    let mut full = String::new();

    for line in std::io::BufReader::new(body.into_reader()).lines() {
        let line = line.map_err(|e| anyhow!("reading the reply: {e}"))?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "transcript.text.delta" => {
                if let Some(delta) = v.get("delta").and_then(Value::as_str) {
                    full.push_str(delta);
                    on_text(&full);
                }
            }
            // The finished transcript, which replaces whatever the deltas
            // spelled out rather than adding to it.
            "transcript.text.done" => {
                if let Some(text) = v.get("text").and_then(Value::as_str) {
                    full = text.to_string();
                }
                on_text(&full);
            }
            _ => {}
        }
    }

    Ok(full.trim().to_string())
}

/// POST the recording, and hand back the reply body for reading.
///
/// The status is checked here rather than at each call site, because a failure
/// has to quote what the endpoint said: among a base URL, a model name, a
/// transport and a key, its own complaint is the only thing that says which one
/// is wrong.
fn post_audio(cfg: &Config, wav: &[u8], stream: bool) -> Result<ureq::Body> {
    let base = cfg.local_base_url.trim();
    if base.is_empty() {
        return Err(anyhow!(
            "No endpoint URL set. Paste one in Settings, under Transcription."
        ));
    }
    // Checked here rather than left to ureq, which reports the same thing as a
    // bare URI parse error with no hint of which setting it came from.
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(anyhow!("The endpoint URL must start with http:// or https://"));
    }

    let url = endpoint(base, "/audio/transcriptions");
    let mut req = ureq::post(&url)
        .config()
        .timeout_global(Some(HTTP_TIMEOUT))
        // The endpoint's own complaint is in the body; a bare status code says
        // nothing about which of the settings is wrong.
        .http_status_as_error(false)
        .build();
    if let Some((name, value)) = bearer(&cfg.local_key) {
        req = req.header(name, &value);
    }

    let (content_type, body) = form(cfg, wav, stream);
    let resp = req
        .header("Content-Type", &content_type)
        .send(body)
        .map_err(|e| anyhow!("could not reach the endpoint at {base}: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        return Ok(resp.into_body());
    }
    let body = resp.into_body().read_to_vec().unwrap_or_default();
    Err(anyhow!(
        "{base} returned HTTP {}: {}",
        status.as_u16(),
        crate::translate::complaint(&body)
    ))
}

/// The upload, and the content type that describes it.
fn form(cfg: &Config, wav: &[u8], stream: bool) -> (String, Vec<u8>) {
    let mut form = Multipart::new(wav);

    // Left out entirely when nothing is set: whisper.cpp's endpoint transcribes
    // with the model it was started with and takes no `model` field, while the
    // OpenAI-shaped ones require it and say so when it is missing.
    let model = cfg.local_model.trim();
    if !model.is_empty() {
        form.text("model", model);
    }
    if let Some(language) = language(cfg) {
        form.text("language", language);
    }
    // JSON, which is what every one of these answers with when asked; without
    // it whisper.cpp's endpoint replies with a bare body instead.
    form.text("response_format", "json");
    if stream {
        form.text("stream", "true");
    }

    // The audio goes last: a parser that walks the body in order has the small
    // fields in hand by the time it reaches the only large one.
    form.file("file", "audio.wav", "audio/wav", wav);
    form.finish()
}

/// A `multipart/form-data` body.
///
/// Written out here rather than taken from ureq's `multipart` feature, which
/// brings a mime-detection crate along with it for what is a boundary, a header
/// per field, and the bytes — and this only ever sends a handful of short values
/// and one file.
struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    /// The boundary is checked against the payload rather than assumed to be
    /// unique: it has 128 bits of clock and pid behind it, so a collision is not
    /// credible, and the check is here so that the failure mode if one ever
    /// happened is a different boundary rather than a corrupt upload.
    fn new(payload: &[u8]) -> Self {
        let mut boundary = String::new();
        for _ in 0..4 {
            boundary = format!("----orra{}", crate::config::random_token());
            if !payload.windows(boundary.len()).any(|w| w == boundary.as_bytes()) {
                break;
            }
        }
        Self { boundary, body: Vec::new() }
    }

    fn text(&mut self, name: &str, value: &str) {
        self.header(&format!("form-data; name=\"{name}\""));
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
    }

    fn file(&mut self, name: &str, file_name: &str, content_type: &str, bytes: &[u8]) {
        // The file name is spelled out because some servers decide how to
        // decode the upload from its extension rather than from the content
        // type, and `blob` is not a format.
        self.header(&format!(
            "form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}"
        ));
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
    }

    fn header(&mut self, disposition: &str) {
        self.body.extend_from_slice(
            format!("--{}\r\nContent-Disposition: {disposition}\r\n\r\n", self.boundary).as_bytes(),
        );
    }

    /// The closing boundary, and the content type the request has to carry to
    /// match what was written.
    fn finish(mut self) -> (String, Vec<u8>) {
        self.body.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (format!("multipart/form-data; boundary={}", self.boundary), self.body)
    }
}

/// The language to steer with, or `None` to leave it to the server.
///
/// `multi` is Deepgram's word for "follow the speaker" and means nothing to
/// these servers; leaving the field out is how each of them spells the same
/// thing — whisper.cpp calls it `auto` and is already there without it.
fn language(cfg: &Config) -> Option<&str> {
    let code = cfg.language.trim();
    (!code.is_empty() && code != crate::deepgram::MULTILINGUAL).then_some(code)
}

/// The text out of a reply.
///
/// Two shapes are read because both are in use: the OpenAI one, `{"text": …}`,
/// and a bare body, which is what whisper.cpp's own endpoint sends.
fn parse_transcript(body: &[u8]) -> Result<String> {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        return match v.get("text").and_then(Value::as_str) {
            Some(text) => Ok(text.trim().to_string()),
            None => Err(anyhow!(
                "the endpoint answered with JSON but no transcript: {}",
                crate::translate::complaint(body)
            )),
        };
    }
    // An empty recording legitimately comes back as an empty body, and
    // `finish_dictation` already treats an empty transcript as nothing to type.
    Ok(String::from_utf8_lossy(body).trim().to_string())
}

/// The URL to post to, from whatever was pasted into the setting.
///
/// The endpoint path is appended unless it is already there, so `…/v1`, `…/v1/`
/// and the full path all end up in the same place. whisper.cpp's `/inference` is
/// the exception the rule cannot cover — it is not an OpenAI path, and pasting it
/// has to work, because it is the only endpoint that server has.
fn endpoint(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with(path) || base.ends_with("/inference") {
        base.to_string()
    } else {
        format!("{base}{path}")
    }
}

/// The auth header the endpoint gets, if a key was given at all.
///
/// A server on this machine usually wants none, and an empty `Bearer ` is what
/// makes some of them refuse the request rather than ignore it.
fn bearer(key: &str) -> Option<(&'static str, String)> {
    let key = key.trim();
    (!key.is_empty()).then(|| ("Authorization", format!("Bearer {key}")))
}

// ---------------------------------------------------------------------------
// the Realtime transport
// ---------------------------------------------------------------------------

/// The local provider's half of the streaming contract.
pub const WIRE: Wire = Wire {
    handshake: session_update,
    // The audio must not follow the session update too closely: on a socket
    // that has not been told it is a transcription session, appended audio is a
    // conversation turn and comes back as a reply rather than a transcript.
    awaits_handshake: true,
    // Committing is what makes the server transcribe what has been sent. The
    // transcript for it arrives after that, which is what `flush` waits for.
    close: &[r#"{"type":"input_audio_buffer.commit"}"#],
    flush: FLUSH,
    frame_samples,
    encode,
    decode,
};

/// The socket URL for the Realtime transport.
///
/// `?model=` is how this protocol names the model — there is no field for it in
/// the session update — and the scheme has to be a socket one, since the URL is
/// most likely pasted as the `http://` the same server answers on.
fn realtime_url(cfg: &Config) -> String {
    let base = cfg.local_base_url.trim().trim_end_matches('/');
    let mut url = if base.ends_with("/realtime") {
        base.to_string()
    } else {
        format!("{base}/realtime")
    };
    if let Some(rest) = url.strip_prefix("https://") {
        url = format!("wss://{rest}");
    } else if let Some(rest) = url.strip_prefix("http://") {
        url = format!("ws://{rest}");
    }

    let model = cfg.local_model.trim();
    if !model.is_empty() {
        url.push_str(&format!("?model={}", crate::stt::url_encode(model)));
    }
    url
}

/// What the socket is told before any audio goes up.
fn session_update(cfg: &Config) -> Vec<String> {
    let mut transcription = json!({});
    if let Some(code) = language(cfg) {
        transcription["language"] = json!(code);
    }

    vec![
        json!({
            "type": "session.update",
            "session": {
                "type": "transcription",
                "audio": {
                    "input": {
                        // Declared rather than assumed: the audio below is
                        // resampled to exactly this.
                        "format": { "type": "audio/pcm", "rate": REALTIME_RATE },
                        "transcription": transcription,
                        // Nothing is committed until the key is released, so
                        // what comes back is the whole dictation instead of a
                        // guess at where the sentences ended while it was still
                        // being spoken.
                        "turn_detection": Value::Null,
                    }
                }
            }
        })
        .to_string(),
    ]
}

/// About a tenth of a second per frame, which is what the API is tuned for.
fn frame_samples(sample_rate: u32) -> usize {
    (sample_rate / 10) as usize
}

fn encode(samples: &[i16], sample_rate: u32) -> Message {
    let pcm = audio::resample_to(samples, sample_rate, REALTIME_RATE);
    let data = STANDARD.encode(audio::i16_to_le_bytes(&pcm));
    Message::Text(
        json!({ "type": "input_audio_buffer.append", "audio": data })
            .to_string()
            .into(),
    )
}

/// Read one Realtime message into the transcript.
fn decode(raw: &str, session: &mut Session) -> Flow {
    let Ok(v) = serde_json::from_str::<Value>(raw) else { return Flow::Continue };

    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        // The acknowledgement. Only now is the server transcribing, which is
        // why `awaits_handshake` holds the audio back until it arrives.
        "session.updated" => session.ready = true,
        // Text as it is decided. `delta` is the new part only, so it is added
        // to the tail rather than replacing it.
        "conversation.item.input_audio_transcription.delta" => {
            if let Some(delta) = v.get("delta").and_then(Value::as_str) {
                session.append_interim(delta);
            }
        }
        "conversation.item.input_audio_transcription.completed" => {
            if let Some(text) = v.get("transcript").and_then(Value::as_str) {
                session.push_final(text, None);
            }
            // The last committed audio has come back, so there is nothing left
            // to wait for and no reason to sit out the flush deadline.
            if session.closing {
                return Flow::Done;
            }
        }
        _ => {}
    }
    Flow::Continue
}

// ---------------------------------------------------------------------------
// the model list
// ---------------------------------------------------------------------------

/// The models a server has, in the order it lists them.
///
/// One shape covers every server worth pointing at, because they all copy
/// OpenAI's: `GET {base}/models` → `{"data":[{"id": …}]}`. Ollama answers it,
/// which is what makes picking one of the models already installed a dropdown
/// rather than a name typed from memory. A server that does not have the route
/// — whisper.cpp's own endpoint — fails here and the model stays free text.
pub fn list_models(base_url: &str, key: &str) -> Result<Vec<String>> {
    let base = base_url.trim();
    if base.is_empty() {
        return Err(anyhow!(
            "No endpoint URL set. Paste one in Settings, under Transcription."
        ));
    }
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(anyhow!("The endpoint URL must start with http:// or https://"));
    }

    let url = models_url(base);
    let mut req = ureq::get(&url)
        .config()
        .timeout_global(Some(crate::problem::VERIFY_TIMEOUT))
        .http_status_as_error(false)
        .build();
    if let Some((name, value)) = bearer(key) {
        req = req.header(name, &value);
    }

    let resp = req.call().map_err(|e| anyhow!("could not reach the endpoint at {base}: {e}"))?;
    let status = resp.status();
    let body = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| anyhow!("reading the model list: {e}"))?;

    if !status.is_success() {
        return Err(anyhow!(
            "{base} returned HTTP {}: {}",
            status.as_u16(),
            crate::translate::complaint(&body)
        ));
    }

    let v: Value = serde_json::from_slice(&body).map_err(|e| {
        anyhow!("{base} did not return JSON ({e}) — is it an OpenAI-compatible API?")
    })?;
    let mut models: Vec<String> = v
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    if models.is_empty() {
        return Err(anyhow!("{base} lists no models"));
    }
    models.sort();
    Ok(models)
}

/// Where a server lists its models, from whatever was pasted.
///
/// Any endpoint path already on the URL comes back off first: `…/v1`, `…/v1/`
/// and `…/v1/audio/transcriptions` are the same server and have to end up at the
/// same list.
fn models_url(base: &str) -> String {
    let mut base = base.trim().trim_end_matches('/');
    for tail in ["/audio/transcriptions", "/inference", "/realtime", "/chat/completions"] {
        if let Some(head) = base.strip_suffix(tail) {
            base = head.trim_end_matches('/');
            break;
        }
    }
    format!("{base}/models")
}

/// The Settings check: hand the server a moment of silence and see it answer.
///
/// A real request rather than a listing, for the same reason the translation
/// check is one: it is the only thing that covers the URL, the port, the
/// transport, the model name and the key together, which are the five things
/// that can be wrong before any audio is sent — and it is the same code path a
/// dictation takes, so a pass here means dictating will work. Asking for a
/// model list instead would fail on every server that does not publish one,
/// which is most of the ones that transcribe with the model they were started
/// with.
pub async fn check(cfg: &Config) -> Result<String> {
    match cfg.local_transport {
        LocalTransport::WebSocket => check_socket(cfg).await,
        // The HTTP transports block, and this is called from the invoke thread.
        _ => {
            let owned = cfg.clone();
            tokio::task::spawn_blocking(move || check_http(&owned))
                .await
                .map_err(|e| anyhow!("the check did not finish: {e}"))?
        }
    }
}

/// Silence, in the shape these servers take audio: 16 kHz mono, half a second.
///
/// Transcribing it proves everything up to the transcript — that the server is
/// there, that it takes this shape of request, and that it will answer. What it
/// answers is not the point, and models are entitled to answer silence with
/// anything at all.
fn silence() -> Vec<u8> {
    audio::wav_bytes(&vec![0i16; (SPEECH_RATE / 2) as usize], SPEECH_RATE)
}

fn check_http(cfg: &Config) -> Result<String> {
    let wav = silence();
    let heard = match cfg.local_transport {
        LocalTransport::Sse => transcribe_sse(cfg, &wav, &mut |_| {})?,
        _ => transcribe_http(cfg, &wav)?,
    };
    // What came back is not judged: silence has no transcript, and a model that
    // writes down something it thought it heard is still a model that answered.
    Ok(if heard.trim().is_empty() {
        "It works — the server transcribed a moment of silence and had nothing to write down."
            .to_string()
    } else {
        format!("It works — the server answered {heard:?}")
    })
}

/// The same check for a server that only speaks the Realtime socket.
///
/// There is no request to make here, so what is proved is a little less: the
/// socket opened, the server took the session it was offered, and it said
/// something back. That covers the address, the scheme and the protocol, which
/// is where this transport goes wrong.
async fn check_socket(cfg: &Config) -> Result<String> {
    let url = realtime_url(cfg);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| anyhow!("{url} is not a URL a socket can be opened on: {e}"))?;
    if let Some((name, value)) = bearer(&cfg.local_key) {
        request.headers_mut().insert(
            name,
            HeaderValue::from_str(&value).map_err(|_| anyhow!("the API key is not a valid header"))?,
        );
    }

    let waited = tokio::time::timeout(crate::problem::VERIFY_TIMEOUT, async {
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| anyhow!("could not open the socket at {url}: {e}"))?;
        for frame in (WIRE.handshake)(cfg) {
            socket
                .send(Message::Text(frame.into()))
                .await
                .map_err(|e| anyhow!("the socket closed before the session was set up: {e}"))?;
        }
        match socket.next().await {
            Some(Ok(Message::Text(text))) => Ok(text.to_string()),
            Some(Ok(Message::Binary(bytes))) => Ok(String::from_utf8_lossy(&bytes).to_string()),
            Some(Ok(_)) => Ok("a frame that was not text".to_string()),
            Some(Err(e)) => Err(anyhow!("the socket failed: {e}")),
            None => Err(anyhow!("the socket closed without a word")),
        }
    })
    .await;

    match waited {
        Ok(Ok(answer)) => {
            let short: String = answer.chars().take(120).collect();
            Ok(format!("It works — the socket answered {short:?}"))
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!(
            "the socket did not answer within {}s",
            crate::problem::VERIFY_TIMEOUT.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A one-shot HTTP stub: answers the first request with `body`, and hands
    /// back what it was sent.
    ///
    /// The request is read until it goes quiet before the reply goes out, since
    /// the client is waiting on that reply rather than closing the connection —
    /// and the multipart body is large enough to arrive in several packets.
    fn stub(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n) = sock.read(&mut buf) {
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let _ = sock.write_all(reply.as_bytes());
            let _ = sock.flush();
            String::from_utf8_lossy(&request).to_string()
        });
        (format!("http://127.0.0.1:{port}/v1"), handle)
    }

    fn configured(base: &str) -> Config {
        Config {
            provider: crate::config::Provider::Local,
            local_base_url: base.to_string(),
            local_model: "whisper-large-v3".into(),
            local_key: "sk-local-test".into(),
            language: "fa".into(),
            ..Config::default()
        }
    }

    fn wav() -> Vec<u8> {
        audio::wav_bytes(&[0i16; 160], SPEECH_RATE)
    }

    #[test]
    fn an_endpoint_url_becomes_the_transcription_path() {
        for (paste, want) in [
            ("http://localhost:8000/v1", "http://localhost:8000/v1/audio/transcriptions"),
            ("http://localhost:8000/v1/", "http://localhost:8000/v1/audio/transcriptions"),
            ("  http://localhost:8000/v1  ", "http://localhost:8000/v1/audio/transcriptions"),
            // Pasted whole, and not appended to twice.
            (
                "http://localhost:8000/v1/audio/transcriptions",
                "http://localhost:8000/v1/audio/transcriptions",
            ),
            // whisper.cpp's own path, which is not an OpenAI one.
            ("http://localhost:8080/inference", "http://localhost:8080/inference"),
            ("http://localhost:8080", "http://localhost:8080/audio/transcriptions"),
        ] {
            assert_eq!(endpoint(paste, "/audio/transcriptions"), want, "paste: {paste}");
        }
    }

    /// Whichever spelling was pasted, the list is asked for at the host's own
    /// `/models` — the endpoint path on it is not part of the question.
    #[test]
    fn the_model_list_is_asked_for_at_the_host_not_at_the_endpoint() {
        for (paste, want) in [
            ("http://localhost:11434/v1", "http://localhost:11434/v1/models"),
            ("http://localhost:11434/v1/", "http://localhost:11434/v1/models"),
            (
                "http://localhost:8000/v1/audio/transcriptions",
                "http://localhost:8000/v1/models",
            ),
            // whisper.cpp's endpoint has no model list, but asking is still
            // better than asking somewhere that does not exist at all.
            ("http://localhost:8080/inference", "http://localhost:8080/models"),
        ] {
            assert_eq!(models_url(paste), want, "paste: {paste}");
        }
    }

    #[test]
    fn the_socket_url_is_a_socket_url() {
        let cfg = |base: &str, model: &str| Config {
            local_base_url: base.into(),
            local_model: model.into(),
            ..Config::default()
        };
        assert_eq!(
            realtime_url(&cfg("http://localhost:8000/v1", "whisper-1")),
            "ws://localhost:8000/v1/realtime?model=whisper-1"
        );
        // The same server written the other way round.
        assert_eq!(
            realtime_url(&cfg("https://speech.example.com/v1", "")),
            "wss://speech.example.com/v1/realtime"
        );
        // A model name with a slash in it is a path as far as a URL is
        // concerned, so it has to be encoded.
        assert_eq!(
            realtime_url(&cfg("http://localhost:8000/v1", "Systran/faster-whisper-large-v3")),
            "ws://localhost:8000/v1/realtime?model=Systran%2Ffaster-whisper-large-v3"
        );
        // Pasting the socket URL itself does not double the path.
        assert_eq!(
            realtime_url(&cfg("ws://localhost:8000/v1/realtime", "")),
            "ws://localhost:8000/v1/realtime"
        );
    }

    #[test]
    fn a_key_is_only_sent_when_there_is_one() {
        assert!(bearer("").is_none());
        assert!(bearer("   ").is_none());
        assert_eq!(
            bearer(" sk-test "),
            Some(("Authorization", "Bearer sk-test".to_string()))
        );
    }

    #[test]
    fn the_language_is_left_to_the_server_when_it_is_multilingual() {
        let mut cfg = configured("http://localhost:8000/v1");
        assert_eq!(language(&cfg), Some("fa"));
        cfg.language = "multi".into();
        assert_eq!(language(&cfg), None, "`multi` means nothing to these servers");
        cfg.language = String::new();
        assert_eq!(language(&cfg), None);
    }

    #[test]
    fn both_reply_shapes_are_read() {
        assert_eq!(parse_transcript(br#"{"text":"  hello there  "}"#).unwrap(), "hello there");
        // whisper.cpp's own endpoint, answering with the transcript alone.
        assert_eq!(parse_transcript(b"  hello there  ").unwrap(), "hello there");
        // A silent recording is not a failure: it is nothing to type.
        assert_eq!(parse_transcript(br#"{"text":""}"#).unwrap(), "");
        // But an error dressed up as a 200 is worth reporting, and the
        // endpoint's own words are the ones that name what is wrong.
        let e = parse_transcript(br#"{"error":{"message":"model not found"}}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("model not found"), "unhelpful: {e}");
    }

    /// The request is the whole feature on this transport, and its shape — the
    /// path, the multipart fields, the header — is the part nothing else
    /// catches. A stub on loopback is the only way to check it without a server.
    #[test]
    fn a_recording_goes_out_and_a_transcript_comes_back() {
        let (base, server) = stub("200 OK", "application/json", r#"{"text":"hello there"}"#);

        let out = transcribe_http(&configured(&base), &wav()).expect("the stub answered");
        assert_eq!(out, "hello there");

        let request = server.join().unwrap().to_lowercase();
        assert!(
            request.starts_with("post /v1/audio/transcriptions "),
            "wrong path: {request}"
        );
        assert!(request.contains("multipart/form-data; boundary="), "not a form: {request}");
        // The audio, under the field name every one of these servers reads.
        assert!(request.contains("name=\"file\"; filename=\"audio.wav\""), "no file: {request}");
        assert!(request.contains("name=\"model\""), "no model: {request}");
        assert!(request.contains("whisper-large-v3"), "wrong model: {request}");
        assert!(request.contains("name=\"language\""), "no language: {request}");
        assert!(request.contains("name=\"response_format\""), "no format: {request}");
        // The key, and the RIFF header of the audio that went with it.
        assert!(request.contains("authorization: bearer sk-local-test"), "no key: {request}");
        assert!(request.contains("riff"), "the upload is not a wav: {request}");
    }

    /// A server with no auth is the normal case on loopback, and an empty
    /// `Bearer ` is what some of them refuse the request over.
    #[test]
    fn a_server_with_no_key_is_not_sent_one() {
        let (base, server) = stub("200 OK", "application/json", r#"{"text":"hi there"}"#);
        let mut cfg = configured(&base);
        cfg.local_key = String::new();

        assert_eq!(transcribe_http(&cfg, &wav()).unwrap(), "hi there");
        let request = server.join().unwrap().to_lowercase();
        assert!(!request.contains("authorization"), "a key was invented: {request}");
    }

    #[test]
    fn a_refused_request_quotes_the_endpoint() {
        let (base, server) = stub(
            "404 Not Found",
            "application/json",
            r#"{"error":{"message":"model not found"}}"#,
        );
        let e = transcribe_http(&configured(&base), &wav()).unwrap_err().to_string();
        assert!(e.contains("404"), "no status: {e}");
        assert!(e.contains("model not found"), "the reason was dropped: {e}");
        let _ = server.join();
    }

    /// The deltas are what the user sees appearing, so the accumulation and the
    /// callback firing are the two things worth pinning.
    #[test]
    fn sse_deltas_accumulate_and_the_final_transcript_wins() {
        let (base, server) = stub(
            "200 OK",
            "text/event-stream",
            "data: {\"type\":\"transcript.text.delta\",\"delta\":\"hello\"}\n\n\
             data: {\"type\":\"transcript.text.delta\",\"delta\":\" there\"}\n\n\
             data: {\"type\":\"transcript.text.done\",\"text\":\"hello there.\"}\n\n\
             data: [DONE]\n\n",
        );

        let mut seen: Vec<String> = Vec::new();
        let out = transcribe_sse(&configured(&base), &wav(), &mut |text| seen.push(text.to_string()))
            .expect("the stub answered");

        assert_eq!(out, "hello there.");
        assert_eq!(seen, vec!["hello", "hello there", "hello there."]);

        let request = server.join().unwrap().to_lowercase();
        assert!(request.contains("name=\"stream\""), "not asked to stream: {request}");
    }

    /// Nothing can be transcribed before the audio exists, so the deltas are
    /// collected in a buffer rather than sent as they are spoken.
    #[test]
    fn the_realtime_wire_reads_the_protocol_it_speaks() {
        let mut session = Session::default();
        assert!(!session.ready, "nothing has been acknowledged yet");

        // The acknowledgement is what lets the audio start.
        assert_eq!(decode(r#"{"type":"session.updated"}"#, &mut session), Flow::Continue);
        assert!(session.ready);

        // Partial text arrives as the new part only.
        decode(
            r#"{"type":"conversation.item.input_audio_transcription.delta","delta":"hello"}"#,
            &mut session,
        );
        decode(
            r#"{"type":"conversation.item.input_audio_transcription.delta","delta":" world"}"#,
            &mut session,
        );
        assert_eq!(session.interim, "hello world");

        // The finished utterance replaces it, and ends the session when the
        // audio has already been committed.
        session.closing = true;
        assert_eq!(
            decode(
                r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"Hello world."}"#,
                &mut session,
            ),
            Flow::Done
        );
        assert_eq!(session.finals, "Hello world.");
        assert_eq!(session.interim, "");

        // Anything else — including a message that is not JSON at all — is not
        // worth ending a dictation over.
        assert_eq!(decode("not json", &mut session), Flow::Continue);
        assert_eq!(decode(r#"{"type":"session.created"}"#, &mut session), Flow::Continue);
    }

    #[test]
    fn the_session_update_declares_what_is_coming() {
        let frames = session_update(&configured("http://localhost:8000/v1"));
        let v: Value = serde_json::from_str(&frames[0]).expect("the update is JSON");
        assert_eq!(v["type"], "session.update");
        assert_eq!(v["session"]["type"], "transcription");
        // The rate the audio is resampled to, declared rather than assumed.
        assert_eq!(v["session"]["audio"]["input"]["format"]["rate"], REALTIME_RATE);
        assert_eq!(v["session"]["audio"]["input"]["format"]["type"], "audio/pcm");
        assert_eq!(v["session"]["audio"]["input"]["transcription"]["language"], "fa");
        // Nothing is committed until the key is released.
        assert!(v["session"]["audio"]["input"]["turn_detection"].is_null());

        // With no language set, the field is left out for the server to detect.
        let mut cfg = configured("http://localhost:8000/v1");
        cfg.language = "multi".into();
        let v: Value = serde_json::from_str(&session_update(&cfg)[0]).unwrap();
        assert!(v["session"]["audio"]["input"]["transcription"].get("language").is_none());
    }

    /// The audio has to reach the socket as the rate the session update
    /// promised, or it is heard as the wrong pitch and length.
    #[test]
    fn encoded_audio_is_the_rate_that_was_declared() {
        let samples = vec![1_000i16; 4_800];
        let Message::Text(frame) = encode(&samples, 48_000) else {
            panic!("the Realtime API takes JSON")
        };
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["type"], "input_audio_buffer.append");

        let audio = STANDARD
            .decode(v["audio"].as_str().expect("base64 audio"))
            .expect("valid base64");
        // 4800 samples at 48 kHz is a tenth of a second, which at the declared
        // 24 kHz is 2400 samples of two bytes each.
        assert_eq!(audio.len(), 2_400 * 2);
        assert_eq!(&audio[0..2], &1_000i16.to_le_bytes());
    }

    #[test]
    fn the_model_list_is_read_and_sorted() {
        let (base, server) = stub(
            "200 OK",
            "application/json",
            r#"{"object":"list","data":[{"id":"whisper-large-v3"},{"id":"base.en"}]}"#,
        );

        let models = list_models(&base, "sk-local-test").expect("the stub answered");
        assert_eq!(models, vec!["base.en", "whisper-large-v3"]);

        let request = server.join().unwrap().to_lowercase();
        assert!(request.starts_with("get /v1/models "), "wrong path: {request}");
        assert!(request.contains("authorization: bearer sk-local-test"), "no key: {request}");
    }

    /// The button that fills the dropdown has to say something useful when the
    /// server has no such route — whisper.cpp's endpoint is the one that does
    /// not — rather than reporting a bare status code.
    #[test]
    fn a_server_with_no_model_route_says_so() {
        let (base, server) = stub("404 Not Found", "text/plain", "not found");
        let e = list_models(&base, "").unwrap_err().to_string();
        assert!(e.contains("404"), "no status: {e}");
        assert!(e.contains("not found"), "the reason was dropped: {e}");
        let _ = server.join();
    }

    /// A plain-text reply is not an OpenAI-compatible list, and saying so is
    /// more use than a JSON parse error.
    #[test]
    fn a_server_that_is_not_openai_compatible_says_so() {
        let (base, server) = stub("200 OK", "text/html", "<html>hello</html>");
        let e = list_models(&base, "").unwrap_err().to_string();
        assert!(e.contains("OpenAI-compatible"), "unhelpful: {e}");
        let _ = server.join();
    }

    /// Before anything is sent: the setting with no usable default, and the
    /// paste ureq would otherwise report as an opaque URI parse error.
    #[test]
    fn a_setting_that_is_missing_is_named_before_anything_is_sent() {
        let e = transcribe_http(&Config::default(), &wav()).unwrap_err().to_string();
        assert!(e.contains("No endpoint URL set"), "unhelpful: {e}");

        let mut cfg = configured("localhost:8000/v1");
        cfg.local_base_url = "localhost:8000/v1".into();
        let e = transcribe_http(&cfg, &wav()).unwrap_err().to_string();
        assert!(e.contains("must start with http://"), "unhelpful: {e}");

        // The same two guards on the model list, which the Settings button
        // reaches with whatever is in the field at that moment.
        let e = list_models("  ", "").unwrap_err().to_string();
        assert!(e.contains("No endpoint URL set"), "unhelpful: {e}");
        let e = list_models("localhost:8000/v1", "").unwrap_err().to_string();
        assert!(e.contains("must start with http://"), "unhelpful: {e}");
    }
}
