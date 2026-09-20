//! Gemini: live transcription over its `BidiGenerateContent` socket.
//!
//! Audio goes up as base64 inside JSON rather than as raw frames, the model has
//! to be named in a setup message the server acknowledges before it will take
//! any audio, and the transcript comes back on its own fields rather than as a
//! model reply — the dedicated transcription model does not answer back.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use crate::audio;
use crate::stt::{url_encode, Flow, Session, Wire};

/// The streaming speech-to-text model, named in [`SETUP`]. Not a setting: this
/// is the one model that behaves like a transcription socket instead of a chat.
const MODEL: &str = "models/gemini-3.5-transcribe-live";

/// Sent the moment the socket opens, and required before any audio — the server
/// rejects audio that arrives before it has acknowledged this.
///
/// `VERBATIM` is the mode for dictation: it writes down what was said rather
/// than tidying it up. An empty `languageCodes` leaves the language to be
/// detected, which is the streaming equivalent of the multilingual mode the
/// Deepgram path defaults to.
const SETUP: &str = r#"{"setup":{"model":"models/gemini-3.5-transcribe-live","generationConfig":{"responseModalities":["TEXT"]},"inputAudioTranscription":{"languageCodes":[],"mode":"VERBATIM"}}}"#;

pub const WIRE: Wire = Wire {
    handshake: |_| vec![SETUP.to_string()],
    awaits_handshake: true,
    // Asking for the end of the audio is what makes the server finalize the
    // open turn; it answers with the last transcript before the socket closes.
    close: &[r#"{"realtimeInput":{"audioStreamEnd":true}}"#],
    flush: crate::stt::FLUSH_TIMEOUT,
    frame_samples,
    encode,
    decode,
};

/// The key travels in the query string here, not in a header.
pub fn url(key: &str) -> String {
    format!(
        "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key={}",
        url_encode(key)
    )
}

/// JSON and base64 cost real bytes per message, and the API asks for around a
/// tenth of a second of audio at a time rather than whatever the sound card
/// hands over.
fn frame_samples(sample_rate: u32) -> usize {
    (sample_rate / 10) as usize
}

fn encode(samples: &[i16], sample_rate: u32) -> Message {
    let data = STANDARD.encode(audio::i16_to_le_bytes(samples));
    Message::Text(
        json!({
            "realtimeInput": {
                "audio": {
                    "data": data,
                    "mimeType": format!("audio/pcm;rate={sample_rate}"),
                }
            }
        })
            .to_string()
            .into(),
    )
}

/// Read one Gemini message into the transcript.
fn decode(raw: &str, session: &mut Session) -> Flow {
    let Ok(v) = serde_json::from_str::<Value>(raw) else { return Flow::Continue };

    if v.get("setupComplete").is_some() {
        session.ready = true;
        return Flow::Continue;
    }

    let Some(content) = v.get("serverContent") else { return Flow::Continue };

    // The finished transcript for a turn is the one to type; the interim field
    // is the same sentence still being spoken. A message can carry either, and
    // the API does not promise which comes first.
    let finished = content.pointer("/inputTranscription/text").and_then(Value::as_str);
    let interim = content.pointer("/interimInputTranscription/text").and_then(Value::as_str);
    if let Some(text) = finished {
        session.push_final(text, None);
    } else if let Some(text) = interim {
        session.set_interim(text);
    }

    // The end of a turn. `generationComplete` is what this model sends once it
    // has nothing left to do with the audio — the last transcript arrives just
    // before it — and `turnComplete` is the other spelling of the same thing.
    //
    // Either only matters once the stream has been asked to close: before that,
    // a turn ending is just the speaker pausing.
    let ended = content.get("generationComplete").is_some() || content.get("turnComplete").is_some();
    if session.closing && ended {
        return Flow::Done;
    }
    Flow::Continue
}

/// Cheap round trip that proves the key is accepted, for the Settings button.
///
/// It also checks the one thing a valid key does not imply: that this key can
/// see the transcription model, which is newer than the models most keys were
/// made for.
pub fn verify_key(key: &str) -> Result<String> {
    let resp = ureq::get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=200")
        .config()
        .timeout_global(Some(crate::problem::VERIFY_TIMEOUT))
        .build()
        .header("x-goog-api-key", key)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(400 | 401 | 403) => anyhow!("Gemini rejected that key ({e})"),
            ureq::Error::Timeout(_) => anyhow!("Gemini did not answer in time"),
            other => anyhow!("could not reach Gemini: {other}"),
        })?;

    let body = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| anyhow!("reading response: {e}"))?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| anyhow!("unexpected response: {e}"))?;

    let has_model = v
        .get("models")
        .and_then(Value::as_array)
        .map(|models| models.iter().any(|m| m.get("name").and_then(Value::as_str) == Some(MODEL)))
        .unwrap_or(false);

    if !has_model {
        return Ok(format!(
            "Key is valid, but it cannot see {} — live transcription may not be available on it",
            MODEL.trim_start_matches("models/")
        ));
    }
    Ok("Key is valid — Gemini accepted it".to_string())
}

#[cfg(test)]
mod tests {
    // Setup mutates a field on `Session::default()`; the struct-literal form
    // says less about which field the test is about.
    #![allow(clippy::field_reassign_with_default)]

    use super::*;

    #[test]
    fn the_setup_names_the_model_it_is_paired_with() {
        // The two are written out separately; this is what keeps them together.
        assert!(SETUP.contains(MODEL), "SETUP and MODEL have drifted apart");
        assert!(SETUP.contains("VERBATIM"));
    }

    #[test]
    fn a_turn_is_read_into_the_transcript() {
        let mut s = Session::default();

        // Audio is refused until this arrives.
        assert!(!s.ready);
        assert_eq!(decode(r#"{"setupComplete":{}}"#, &mut s), Flow::Continue);
        assert!(s.ready);

        decode(r#"{"serverContent":{"interimInputTranscription":{"text":"hello wor"}}}"#, &mut s);
        assert_eq!(s.interim, "hello wor");
        assert!(s.finals.is_empty());

        decode(r#"{"serverContent":{"inputTranscription":{"text":"Hello world."}}}"#, &mut s);
        assert_eq!(s.finals, "Hello world.");
        assert!(s.interim.is_empty());
    }

    #[test]
    fn a_finished_turn_only_ends_the_stream_once_it_has_been_asked_to() {
        // Both spellings the socket has been seen to use for "the turn is over",
        // with the one this model actually sends first.
        for end in [
            r#"{"serverContent":{"generationComplete":true}}"#,
            r#"{"serverContent":{"turnComplete":true}}"#,
        ] {
            let mut s = Session::default();

            // The speaker pausing is not the end of the dictation.
            assert_eq!(decode(end, &mut s), Flow::Continue);

            s.closing = true;
            assert_eq!(decode(end, &mut s), Flow::Done);
        }
    }

    /// The order the socket really uses at the end of a dictation: the last
    /// transcript, then the signal that there is nothing more. Read the other
    /// way round the words would be thrown away.
    #[test]
    fn the_last_transcript_arrives_before_the_end_signal() {
        let mut s = Session::default();
        s.closing = true;

        decode(r#"{"serverContent":{"inputTranscription":{"text":"Done."}}}"#, &mut s);
        assert_eq!(s.finals, "Done.");
        assert_eq!(decode(r#"{"serverContent":{"generationComplete":true}}"#, &mut s), Flow::Done);
    }

    #[test]
    fn audio_is_sent_as_base64_pcm_at_the_declared_rate() {
        let Message::Text(frame) = encode(&[1, -1], 16_000) else { panic!("not text") };
        let v: Value = serde_json::from_str(&frame).unwrap();
        let audio = v.pointer("/realtimeInput/audio").expect("audio object");
        assert_eq!(audio["mimeType"], "audio/pcm;rate=16000");
        // Little-endian 1, -1.
        assert_eq!(STANDARD.decode(audio["data"].as_str().unwrap()).unwrap(), vec![1, 0, 0xFF, 0xFF]);
    }

    /// Frames arrive as JSON, but not always as *text* frames — the Live API
    /// answers in binary ones, which is worth knowing before writing a reader.
    fn text_of(m: Message) -> Option<String> {
        match m {
            Message::Text(t) => Some(t.to_string()),
            Message::Binary(b) => Some(String::from_utf8_lossy(&b).to_string()),
            _ => None,
        }
    }

    /// Speak a sentence with Aura, stream it up the live socket, and print
    /// everything the server says back.
    ///
    /// The one thing the documentation leaves ambiguous is which message
    /// carries the transcript and how many of them there are per turn, which is
    /// exactly what a dictation depends on — so this reads it off the wire.
    ///
    /// Ignored by default. Run with:
    ///   `cargo test live_socket -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "hits the Gemini API; run with --ignored"]
    async fn live_socket_round_trip() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        use crate::config::{Config, Provider};

        let cfg = Config::default();
        let key = cfg.key_for(Provider::Gemini).expect("GEMINI_API_KEY must be set");
        let dg_key = cfg.key_for(Provider::Deepgram).expect("DEEPGRAM_API_KEY must be set");

        // 16 kHz, so the samples go out exactly as the socket wants them.
        let wav = crate::deepgram::speak_wav(
            &dg_key,
            "aura-2-thalia-en",
            "Orra is transcribing this sentence with Gemini.",
            16_000,
        )
        .expect("synthesis failed");
        let samples = crate::deepgram::read_wav_pcm16(&wav);
        assert!(samples.len() > 16_000, "expected about a second of audio");

        let request = url(&key).as_str().into_client_request().unwrap();
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("could not open the live socket");
        let (mut write, mut read) = ws.split();

        for frame in (WIRE.handshake)(&crate::config::Config::default()) {
            write.send(Message::Text(frame.into())).await.unwrap();
        }

        // Wait for the setup acknowledgement before sending audio.
        let mut session = Session::default();
        let ready = tokio::time::sleep(std::time::Duration::from_secs(10));
        tokio::pin!(ready);
        while !session.ready {
            tokio::select! {
                frame = read.next() => match frame {
                    Some(Ok(m)) => {
                        if let Some(t) = text_of(m) {
                            println!("<< {t}");
                            decode(&t, &mut session);
                        } else {
                            println!("?? a frame that was not text");
                        }
                    }
                    Some(Err(e)) => { println!("!! {e}"); break }
                    None => { println!("-- closed before the setup was acknowledged"); break }
                },
                _ = &mut ready => { println!("-- no acknowledgement in 10s"); break }
            }
        }

        let chunk = (WIRE.frame_samples)(16_000);
        let mut sent = 0;
        for block in samples.chunks(chunk) {
            let mut frame = block.to_vec();
            frame.resize(chunk, 0);
            write.send(encode(&frame, 16_000)).await.unwrap();
            sent += 1;
            // Real-time pace, as a microphone would.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        println!(">> {sent} frames of audio, then audioStreamEnd");
        session.closing = true;
        for frame in WIRE.close {
            write.send(Message::Text((*frame).into())).await.unwrap();
        }

        let deadline = tokio::time::sleep(std::time::Duration::from_secs(15));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                frame = read.next() => match frame {
                    Some(Ok(m)) => {
                        if let Some(t) = text_of(m) {
                            println!("<< {t}");
                            if decode(&t, &mut session) == Flow::Done {
                                println!("-- ended on the provider's own signal");
                                break;
                            }
                        }
                    }
                    Some(Err(e)) => { println!("!! {e}"); break }
                    None => { println!("-- socket closed"); break }
                },
                _ = &mut deadline => { println!("-- timed out"); break }
            }
        }

        println!("finals: {:?}", session.finals);
        println!("interim: {:?}", session.interim);
        assert!(!session.finals.is_empty(), "nothing was transcribed");
    }

    #[test]
    fn the_key_is_encoded_into_the_url() {
        let u = url("a+b/c=");
        assert!(u.starts_with("wss://generativelanguage.googleapis.com/ws/"));
        assert!(u.ends_with("?key=a%2Bb%2Fc%3D"), "got {u}");
    }
}
