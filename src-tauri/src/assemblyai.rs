//! AssemblyAI: streaming speech-to-text (the v3 "Universal Streaming" socket).
//!
//! The streaming host is not the REST one, and it takes its key as a plain
//! `Authorization` header rather than the `Token …` form Deepgram wants — see
//! [`crate::stt::start`], which builds the request.

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

use crate::audio;
use crate::stt::{Flow, Session, Wire};

pub const WIRE: Wire = Wire {
    // All the configuration is in the URL, as with Deepgram.
    handshake: |_| Vec::new(),
    awaits_handshake: false,
    flush: crate::stt::FLUSH_TIMEOUT,
    // Closing the socket as soon as this goes out would discard the last
    // transcript: the server flushes the open turn first and answers with a
    // `Termination` message, which is what `decode` waits for.
    close: &[r#"{"type":"Terminate"}"#],
    frame_samples,
    encode,
    decode,
};

/// The languages the socket will steer towards.
///
/// Anything outside this list — Persian, for one — is not supported by the
/// streaming models at all; AssemblyAI only transcribes it through its
/// pre-recorded endpoint. An unrecognised code is *ignored* rather than
/// rejected, so sending one would look like it worked and transcribe in the
/// wrong language. This list is what keeps that from happening quietly.
pub const LANGUAGES: &[&str] = &[
    "en", "es", "fr", "de", "it", "pt", "tr", "nl", "sv", "no", "da", "fi", "hi", "vi", "ar",
    "he", "ja", "zh",
];

/// The socket describes the audio rather than sniffing it, and defaults to
/// 16 kHz — which would silently mis-transcribe a 48 kHz device, since the
/// rate is taken from the microphone and simply echoed here.
///
/// `language` steers the model. It is sent as a JSON array in the query string,
/// which is how the API takes it — `language_codes=["es"]`, not `language=es`.
pub fn url(sample_rate: u32, language: &str) -> String {
    let mut url = format!(
        "wss://streaming.assemblyai.com/v3/ws?encoding=pcm_s16le&sample_rate={sample_rate}&format_turns=true"
    );

    let code = language.trim().to_ascii_lowercase();
    if LANGUAGES.contains(&code.as_str()) {
        let list = crate::stt::url_encode(&format!("[\"{code}\"]"));
        url.push_str(&format!("&language_codes={list}"));
    }
    url
}

/// Frames have to be 50–1000 ms of audio; anything smaller closes the session
/// with 3007. A sound card hands over about ten milliseconds at a time, so the
/// loop gathers them into tenths of a second before sending.
fn frame_samples(sample_rate: u32) -> usize {
    (sample_rate / 10) as usize
}

fn encode(samples: &[i16], _sample_rate: u32) -> Message {
    Message::Binary(audio::i16_to_le_bytes(samples).into())
}

/// Read one AssemblyAI message into the transcript.
fn decode(raw: &str, session: &mut Session) -> Flow {
    let Ok(v) = serde_json::from_str::<Value>(raw) else { return Flow::Continue };

    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "Turn" => {
            let transcript = v.get("transcript").and_then(Value::as_str).unwrap_or("");
            let ended = v.get("end_of_turn").and_then(Value::as_bool).unwrap_or(false);
            // With `format_turns` on, a finished turn arrives twice: the raw
            // transcript, then the punctuated one. Only the second is the turn;
            // taking both would type everything out twice.
            let formatted = v.get("turn_is_formatted").and_then(Value::as_bool).unwrap_or(false);
            // Confidence rides on the turn, not on the transcript.
            let confidence = v
                .get("end_of_turn_confidence")
                .and_then(Value::as_f64)
                .map(|c| c as f32);

            if ended && formatted {
                session.push_final(transcript, confidence);
            } else {
                session.set_interim(transcript);
            }
            Flow::Continue
        }
        // The last message of a session, sent once everything has been flushed.
        "Termination" => Flow::Done,
        // Sent immediately before the close frame, whose own reason is truncated
        // to 123 bytes and usually reads "See Error message for details".
        "Error" => {
            let message = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the transcription service reported an error");
            session.set_interim(message);
            Flow::Done
        }
        _ => Flow::Continue,
    }
}

/// Cheap round trip that proves the key is accepted, for the Settings button.
///
/// The streaming host has no endpoint for this, so it asks the REST one, which
/// takes the same key in the same header.
pub fn verify_key(key: &str) -> Result<String> {
    ureq::get("https://api.assemblyai.com/v2/transcript?limit=1")
        .config()
        .timeout_global(Some(crate::problem::VERIFY_TIMEOUT))
        .build()
        .header("Authorization", key)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => anyhow!("AssemblyAI rejected that key (401)"),
            ureq::Error::StatusCode(code) => anyhow!("AssemblyAI returned HTTP {code}"),
            ureq::Error::Timeout(_) => anyhow!("AssemblyAI did not answer in time"),
            other => anyhow!("could not reach AssemblyAI: {other}"),
        })?;
    Ok("Key is valid — AssemblyAI accepted it".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole turn at each stage of its life, as the socket sends it.
    #[test]
    fn only_the_formatted_end_of_a_turn_becomes_final() {
        let mut s = Session::default();

        s.set_interim("hello wor");
        assert!(s.finals.is_empty());

        s.set_interim("Hello world");
        assert_eq!(s.interim, "Hello world");
        assert!(s.finals.is_empty());

        // The turn ends, unformatted: still not the text to type.
        assert_eq!(
            decode(
                r#"{"type":"Turn","turn_order":0,"end_of_turn":true,"turn_is_formatted":false,"transcript":"hello world"}"#,
                &mut s
            ),
            Flow::Continue
        );
        assert!(s.finals.is_empty());

        // The formatted turn that follows it is.
        assert_eq!(
            decode(
                r#"{"type":"Turn","turn_order":0,"end_of_turn":true,"turn_is_formatted":true,"transcript":"Hello, world.","end_of_turn_confidence":0.94}"#,
                &mut s
            ),
            Flow::Continue
        );
        assert_eq!(s.finals, "Hello, world.");
        assert!(s.interim.is_empty());
        assert_eq!(s.confidence.mean(), Some(0.94));
    }

    #[test]
    fn turns_accumulate_and_termination_ends_the_stream() {
        let mut s = Session::default();
        for t in ["One.", "Two."] {
            decode(
                &format!(
                    r#"{{"type":"Turn","end_of_turn":true,"turn_is_formatted":true,"transcript":"{t}"}}"#
                ),
                &mut s,
            );
        }
        assert_eq!(s.finals, "One. Two.");

        assert_eq!(decode(r#"{"type":"Termination","audio_duration_seconds":3}"#, &mut s), Flow::Done);
        // Nothing else in the protocol ends it.
        assert_eq!(decode(r#"{"type":"Begin","id":"abc"}"#, &mut s), Flow::Continue);
    }

    #[test]
    fn a_supported_language_is_sent_as_a_json_array() {
        let u = url(16_000, "es");
        assert!(u.contains("language_codes=%5B%22es%22%5D"), "got {u}");
        // Case and padding are the settings' business, not the API's.
        assert!(url(16_000, " ES ").contains("language_codes=%5B%22es%22%5D"));
    }

    #[test]
    fn a_language_the_socket_cannot_steer_towards_is_not_sent() {
        // Persian is not a streaming language, and `multi` is Deepgram's word
        // for "no language chosen". Sending either would be ignored by the
        // socket, so the session would quietly transcribe in the wrong language.
        for unsupported in ["fa", "multi", "", "  ", "xx", "en-US"] {
            let u = url(16_000, unsupported);
            assert!(!u.contains("language_codes"), "{unsupported:?} was sent: {u}");
        }
    }

    #[test]
    fn the_supported_list_is_the_one_the_api_documents() {
        assert_eq!(LANGUAGES.len(), 18);
        assert!(LANGUAGES.contains(&"en"));
        assert!(LANGUAGES.contains(&"ar"));
        // The two the user dictates in most: one is supported, one is not.
        assert!(!LANGUAGES.contains(&"fa"));
        for code in LANGUAGES {
            assert_eq!(code.len(), 2, "{code} is not an ISO 639-1 code");
        }
    }

    #[test]
    fn the_url_states_the_audio_format_or_the_rate_is_assumed() {
        let u = url(48_000, "");
        assert!(u.starts_with("wss://streaming.assemblyai.com/v3/ws?"));
        assert!(u.contains("encoding=pcm_s16le"));
        assert!(u.contains("sample_rate=48000"));
    }

    #[test]
    fn frames_are_a_tenth_of_a_second_and_never_below_the_minimum() {
        // The protocol rejects anything under 50 ms.
        for rate in [8_000, 16_000, 44_100, 48_000] {
            let samples = frame_samples(rate);
            let ms = samples as f64 * 1000.0 / rate as f64;
            assert!((50.0..=1000.0).contains(&ms), "{rate} Hz gives {ms} ms");
        }
    }
}
