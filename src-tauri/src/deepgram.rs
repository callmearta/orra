//! Deepgram: streaming speech-to-text over WebSocket, and text-to-speech over REST.
//!
//! Only the Deepgram-shaped half of the streaming contract lives here — the
//! URL, the frames, how a result is read. The session loop that drives it is in
//! [`crate::stt`], which is where the other providers plug into the same one.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use crate::audio;
use crate::config::Config;
use crate::stt::{url_encode, Flow, Session, Wire};

/// Deepgram's half of the streaming contract.
pub const WIRE: Wire = Wire {
    // Nothing to set up: the parameters are all in the URL and the socket is
    // ready for audio the moment it opens.
    handshake: &[],
    awaits_handshake: false,
    close: &[r#"{"type":"CloseStream"}"#],
    frame_samples: no_framing,
    encode,
    decode,
};

/// Deepgram takes frames of any size, so the device's own blocks go straight
/// out — which is also the lowest latency to the first interim result.
fn no_framing(_sample_rate: u32) -> usize {
    0
}

fn encode(samples: &[i16], _sample_rate: u32) -> Message {
    Message::Binary(audio::i16_to_le_bytes(samples).into())
}

/// Read one Deepgram message into the transcript.
fn decode(raw: &str, session: &mut Session) -> Flow {
    let Ok(v) = serde_json::from_str::<Value>(raw) else { return Flow::Continue };

    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "Results" => {
            let alt = v
                .pointer("/channel/alternatives/0")
                .cloned()
                .unwrap_or(Value::Null);
            let transcript = alt.get("transcript").and_then(Value::as_str).unwrap_or("");
            // Deepgram reports confidence per alternative; a response without
            // one simply does not contribute to the mean.
            let confidence = alt.get("confidence").and_then(Value::as_f64).map(|c| c as f32);
            if v.get("is_final").and_then(Value::as_bool).unwrap_or(false) {
                session.push_final(transcript, confidence);
            } else {
                session.set_interim(transcript);
            }
            Flow::Continue
        }
        // Deepgram signals it has flushed everything it is going to send.
        "Metadata" => Flow::Done,
        _ => Flow::Continue,
    }
}

#[derive(Debug, Clone)]
pub struct SttOptions {
    pub model: String,
    pub language: String,
    pub smart_format: bool,
    pub keyterms: Vec<String>,
}

impl SttOptions {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            model: cfg.stt_model.clone(),
            language: cfg.language.clone(),
            smart_format: cfg.smart_format,
            keyterms: cfg.dictionary.clone(),
        }
    }
}

/// Nova-3's multilingual mode: transcribes and follows switching between the
/// languages it supports (English, Spanish, French, German, Hindi, Russian,
/// Portuguese, Japanese, Italian, Dutch). Used when no language is configured.
pub const MULTILINGUAL: &str = "multi";

pub fn build_url(opts: &SttOptions, sample_rate: u32) -> String {
    let mut q = vec![
        ("model", opts.model.clone()),
        ("encoding", "linear16".into()),
        ("sample_rate", sample_rate.to_string()),
        ("channels", "1".into()),
        ("interim_results", "true".into()),
        ("punctuate", "true".into()),
        ("smart_format", opts.smart_format.to_string()),
        ("endpointing", "300".into()),
        ("vad_events", "true".into()),
        ("utterance_end_ms", "1000".into()),
    ];

    // Deepgram's `detect_language` is batch-only — the streaming socket answers
    // "Language detection is only supported for batch." with a 400. So the
    // closest thing to automatic here is nova-3's multilingual mode, which is
    // what an unset language falls back to.
    let language = match opts.language.trim() {
        "" => MULTILINGUAL,
        other => other,
    };
    q.push(("language", language.to_string()));

    // nova-3 and flux take plain keyterms; older models take weighted keywords.
    let modern = opts.model.starts_with("nova-3") || opts.model.starts_with("flux");
    for term in &opts.keyterms {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        if modern {
            q.push(("keyterm", term.to_string()));
        } else {
            q.push(("keywords", format!("{term}:2")));
        }
    }

    let query = q
        .iter()
        .map(|(k, v)| format!("{k}={}", url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("wss://api.deepgram.com/v1/listen?{query}")
}

/// Pull 16-bit mono samples out of a canonical WAV file.
#[cfg(test)]
pub(crate) fn read_wav_pcm16(wav: &[u8]) -> Vec<i16> {
    // Walk the chunks rather than assuming a 44-byte header: encoders are
    // free to emit LIST/fact chunks before `data`.
    let mut pos = 12;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len = u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]])
            as usize;
        let body = pos + 8;
        if id == b"data" {
            let end = (body + len).min(wav.len());
            return wav[body..end]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
        }
        pos = body + len + (len & 1); // chunks are word-aligned
    }
    panic!("no data chunk in WAV");
}

// ---------------------------------------------------------------------------
// text to speech
// ---------------------------------------------------------------------------

/// Deepgram caps a single speak request, so long text is split on sentence ends.
const TTS_CHUNK: usize = 1800;

pub fn chunk_for_speech(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if text.chars().count() <= TTS_CHUNK {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        if current.chars().count() + sentence.chars().count() > TTS_CHUNK && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(&sentence);
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if ".!?\n".contains(c) {
            // Keep trailing spaces and quotes with the sentence they close.
            while matches!(chars.peek(), Some(' ') | Some('"') | Some('\'')) {
                current.push(chars.next().unwrap());
            }
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Cheap round trip that proves the key is accepted, for the Settings button.
pub fn verify_key(key: &str) -> Result<String> {
    // Bounded, so a request that connects and then stalls fails the check
    // instead of leaving the button saying "Checking…" until the app is closed.
    let resp = ureq::get("https://api.deepgram.com/v1/projects")
        .config()
        .timeout_global(Some(crate::problem::VERIFY_TIMEOUT))
        .build()
        .header("Authorization", &format!("Token {key}"))
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => anyhow!("Deepgram rejected that key (401)"),
            ureq::Error::StatusCode(code) => anyhow!("Deepgram returned HTTP {code}"),
            ureq::Error::Timeout(_) => anyhow!("Deepgram did not answer in time"),
            other => anyhow!("could not reach Deepgram: {other}"),
        })?;

    let body = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| anyhow!("reading response: {e}"))?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| anyhow!("unexpected response: {e}"))?;
    let name = v
        .pointer("/projects/0/name")
        .and_then(Value::as_str)
        .unwrap_or("your Deepgram project");
    Ok(format!("Key is valid — {name}"))
}

/// Playback rate for synthesized speech.
pub const TTS_RATE: u32 = 24_000;

/// Synthesize one chunk to WAV bytes. Blocking — call from a worker thread.
pub fn speak_chunk(key: &str, model: &str, text: &str) -> Result<Vec<u8>> {
    speak_wav(key, model, text, TTS_RATE)
}

/// Ask for a rate the caller can use directly. The round-trip test requests
/// 16 kHz so the audio can be fed straight back into the listen socket.
pub fn speak_wav(key: &str, model: &str, text: &str, sample_rate: u32) -> Result<Vec<u8>> {
    let url = format!(
        "https://api.deepgram.com/v1/speak?model={}&encoding=linear16&container=wav&sample_rate={sample_rate}",
        url_encode(model)
    );
    let resp = ureq::post(&url)
        .header("Authorization", &format!("Token {key}"))
        .header("Content-Type", "application/json")
        .send_json(json!({ "text": text }))
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => anyhow!("Deepgram speech returned HTTP {code}"),
            other => anyhow!("Deepgram speech request failed: {other}"),
        })?;

    resp.into_body()
        .read_to_vec()
        .map_err(|e| anyhow!("reading speech audio: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    fn opts(model: &str, terms: &[&str]) -> SttOptions {
        SttOptions {
            model: model.into(),
            language: "en".into(),
            smart_format: true,
            keyterms: terms.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn url_carries_encoding_and_rate_so_no_resampling_is_needed() {
        let u = build_url(&opts("nova-3", &[]), 48_000);
        assert!(u.starts_with("wss://api.deepgram.com/v1/listen?"));
        assert!(u.contains("encoding=linear16"));
        assert!(u.contains("sample_rate=48000"));
        assert!(u.contains("channels=1"));
        assert!(u.contains("interim_results=true"));
    }

    /// Which parameters a URL actually carries, so tests do not have to worry
    /// about one name being a substring of another (`language` vs
    /// `detect_language`).
    fn params(url: &str) -> Vec<(String, String)> {
        url.split_once('?')
            .map(|(_, q)| {
                q.split('&')
                    .filter_map(|kv| kv.split_once('='))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn get<'a>(ps: &'a [(String, String)], key: &str) -> Vec<&'a str> {
        ps.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
    }

    #[test]
    fn an_unset_language_falls_back_to_multilingual() {
        let mut o = opts("nova-3", &[]);
        o.language = String::new();
        assert_eq!(get(&params(&build_url(&o, 16_000)), "language"), vec!["multi"]);

        o.language = "   ".into();
        assert_eq!(get(&params(&build_url(&o, 16_000)), "language"), vec!["multi"]);

        // A real code is passed straight through, including non-Latin markets.
        o.language = "fa".into();
        assert_eq!(get(&params(&build_url(&o, 16_000)), "language"), vec!["fa"]);
    }

    #[test]
    fn detect_language_is_never_sent() {
        // The streaming endpoint rejects it with a 400 ("only supported for
        // batch"), which would break every dictation — guard against a regression.
        let mut o = opts("nova-3", &[]);
        for lang in ["multi", "en", "fa", ""] {
            o.language = lang.into();
            let ps = params(&build_url(&o, 16_000));
            assert!(get(&ps, "detect_language").is_empty(), "sent for {lang:?}");
        }
    }

    #[test]
    fn the_default_config_uses_nova_3_multilingual() {
        let cfg = crate::config::Config::default();
        assert_eq!(cfg.stt_model, "nova-3");
        assert_eq!(cfg.language, MULTILINGUAL);
        let ps = params(&build_url(&SttOptions::from_config(&cfg), 16_000));
        assert_eq!(get(&ps, "model"), vec!["nova-3"]);
        assert_eq!(get(&ps, "language"), vec!["multi"]);
    }

    #[test]
    fn keyterms_are_encoded_and_use_the_right_parameter() {
        // Spaces are illegal raw in a query string.
        let u = build_url(&opts("nova-3", &["Orra", "hyper land"]), 16_000);
        assert!(u.contains("keyterm=Orra"));
        assert!(u.contains("keyterm=hyper%20land"));
        assert!(!u.contains("keywords="));

        // nova-2 predates `keyterm` and wants weighted keywords instead.
        let old = build_url(&opts("nova-2", &["Orra"]), 16_000);
        assert!(old.contains("keywords=Orra%3A2"));
    }

    #[test]
    fn empty_keyterms_are_skipped() {
        let u = build_url(&opts("nova-3", &["", "  "]), 16_000);
        assert!(!u.contains("keyterm="));
    }

    #[test]
    fn short_text_is_a_single_speech_chunk() {
        assert_eq!(chunk_for_speech("hello there"), vec!["hello there"]);
        assert!(chunk_for_speech("   ").is_empty());
    }

    #[test]
    fn long_text_splits_on_sentence_boundaries_within_the_cap() {
        let sentence = "This is a sentence that runs on for a little while. ";
        let text = sentence.repeat(80); // well over the 1800 character cap
        let chunks = chunk_for_speech(&text);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.chars().count() <= TTS_CHUNK, "chunk exceeded the cap");
            assert!(c.trim_end().ends_with('.'), "chunk should end on a sentence");
        }
        // No text is lost at the seams.
        let rejoined: String = chunks.concat();
        assert_eq!(rejoined.split_whitespace().count(), text.split_whitespace().count());
    }

    fn live_key() -> String {
        crate::config::Config::default()
            .api_key()
            .expect("DEEPGRAM_API_KEY must be set (environment or .env)")
    }

    /// Synthesize a sentence with Aura, stream it back through the listen socket,
    /// and return `(transcript, detected_language)`.
    async fn round_trip(key: &str, opts: SttOptions, spoken: &str) -> (String, String) {
        // 16 kHz so the samples go straight back to the socket with no resampling.
        let wav = speak_wav(key, "aura-2-thalia-en", spoken, 16_000).expect("synthesis failed");
        assert_eq!(&wav[0..4], b"RIFF", "speak should return a WAV container");

        let samples = read_wav_pcm16(&wav);
        assert!(samples.len() > 16_000, "expected about a second of audio");

        let mut request = build_url(&opts, 16_000)
            .as_str()
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Token {key}")).unwrap(),
        );

        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("could not open the listen stream — bad key, model or parameter");
        let (mut write, mut read) = ws.split();

        // Feed it in ~100 ms blocks, as the live capture loop would.
        for block in samples.chunks(1_600) {
            write
                .send(Message::Binary(audio::i16_to_le_bytes(block).into()))
                .await
                .unwrap();
        }
        write
            .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
            .await
            .unwrap();

        let (mut heard, mut detected) = (String::new(), String::new());
        while let Some(Ok(msg)) = read.next().await {
            let Message::Text(t) = msg else { continue };
            let v: Value = serde_json::from_str(&t).unwrap();
            match v.get("type").and_then(Value::as_str) {
                Some("Metadata") => break,
                Some("Error") => panic!("Deepgram rejected the stream: {t}"),
                _ => {}
            }
            if let Some(lang) = v.pointer("/channel/detected_language").and_then(Value::as_str) {
                detected = lang.to_string();
            }
            if v.get("is_final").and_then(Value::as_bool) == Some(true) {
                if let Some(text) = v
                    .pointer("/channel/alternatives/0/transcript")
                    .and_then(Value::as_str)
                {
                    if !heard.is_empty() {
                        heard.push(' ');
                    }
                    heard.push_str(text.trim());
                }
            }
        }
        (heard, detected)
    }

    fn live_opts(language: &str) -> SttOptions {
        SttOptions {
            model: "nova-3".into(),
            language: language.into(),
            smart_format: true,
            keyterms: vec!["Orra".into()],
        }
    }

    /// Ignored by default — these need the network and a live key. Run with:
    ///   `cargo test -- --ignored --nocapture`
    /// A voice command only fires if the words reach `polish` intact. This is
    /// the check that Deepgram really says "new line" back and not something
    /// smart_format decided was tidier — a spoken command that arrives as
    /// "newline" is a command that silently does nothing but get typed.
    #[tokio::test]
    #[ignore = "hits the Deepgram API; run with --ignored"]
    async fn spoken_commands_survive_the_round_trip() {
        let cfg = crate::config::Config::default();

        let (heard, _) = round_trip(&live_key(), live_opts("en"), "first line new line second line").await;
        println!("spoken command heard back: {heard:?}");
        let p = crate::polish::process(&heard, &cfg);
        println!("polished:                   {:?}", p.text);
        assert!(p.text.contains('\n'), "\"new line\" never fired: {heard:?}");
        assert!(
            !p.text.contains("\n,"),
            "a comma was left dangling at the head of the new line: {:?}",
            p.text
        );

        let (heard, _) = round_trip(&live_key(), live_opts("en"), "let us ship this press enter").await;
        println!("submit command heard back: {heard:?}");
        let p = crate::polish::process(&heard, &cfg);
        println!("polished:                  {:?} submit={}", p.text, p.submit);
        assert!(p.submit, "\"press enter\" did not submit: {heard:?}");
    }

    #[tokio::test]
    #[ignore = "hits the Deepgram API; run with --ignored"]
    async fn speech_round_trips_through_both_endpoints() {
        let (heard, _) = round_trip(&live_key(), live_opts("en"), "Orra streaming dictation is working correctly.").await;
        println!("heard back: {heard:?}");
        let lower = heard.to_lowercase();
        assert!(
            lower.contains("orra") && lower.contains("working"),
            "round trip lost the words: {heard:?}"
        );
    }

    /// `language=multi` is nova-3's multilingual streaming mode — the closest
    /// thing the streaming endpoint has to automatic language handling, since
    /// `detect_language` is batch-only.
    #[tokio::test]
    #[ignore = "hits the Deepgram API; run with --ignored"]
    async fn multilingual_mode_transcribes_english() {
        let (heard, _) = round_trip(
            &live_key(),
            live_opts("multi"),
            "Orra streaming dictation is working correctly.",
        )
        .await;
        println!("heard back in multi mode: {heard:?}");
        assert!(
            heard.to_lowercase().contains("orra"),
            "multilingual mode lost the words: {heard:?}"
        );
    }

    #[test]
    fn results_accumulate_finals_and_track_interim() {
        let mut s = Session::default();

        // Interim results must not leak into the final transcript.
        assert_eq!(
            decode(r#"{"type":"Results","is_final":false,"channel":{"alternatives":[{"transcript":"hello wor"}]}}"#, &mut s),
            Flow::Continue
        );
        assert_eq!(s.interim, "hello wor");
        assert!(s.finals.is_empty());

        decode(r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":"Hello world."}]}}"#, &mut s);
        decode(r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":"How are you?"}]}}"#, &mut s);
        assert_eq!(s.finals, "Hello world. How are you?");
        assert!(s.interim.is_empty());

        // The flush is what ends the stream.
        assert_eq!(decode(r#"{"type":"Metadata"}"#, &mut s), Flow::Done);
    }
}
