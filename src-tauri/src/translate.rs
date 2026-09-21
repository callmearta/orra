//! Turning a finished transcript into another language, with an LLM.
//!
//! This runs after the transcript has been cleaned up and before anything is
//! typed, so what lands in the focused window is the translation rather than
//! the words that were spoken. It is the one place a dictation waits on a
//! second network call, which is why it is a deliberate keystroke of its own
//! rather than something every dictation pays for.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::config::{Config, TranslateProvider};

/// How long a translation gets before the dictation gives up on it.
///
/// ureq waits forever by default, and the far end can be a URL someone typed
/// that points at a machine which is switched off, or a server that accepts the
/// connection and then says nothing — an unfinished translation is a dictation
/// that never gets typed at all, where a failed one still types the words that
/// were spoken. Deliberately long: the service may be a model on this machine,
/// where a cold load alone can take a minute before the first token. The wait
/// is visible as the overlay's processing state, so it does not look like
/// nothing is happening.
///
/// The Settings check takes the same path with [`crate::problem::VERIFY_TIMEOUT`
/// ] instead — nobody is waiting on a dictation, but somebody is waiting on a
/// button.
const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(120);

/// What the model is asked for. Translation only — no summaries, no answers,
/// no explaining — because whatever comes back is typed out verbatim.
fn prompt(target: &str, text: &str) -> String {
    format!(
        "Translate the dictation below into {target}. Output only the translation: no \
         commentary, no quotes, no preamble. Use exactly the line breaks of the \
         original — neither more nor fewer, and do not break lines at sentence ends. \
         Keep its punctuation.\n\n{text}"
    )
}

/// Translate `text` into the configured language. Blocking — call it from a
/// worker thread.
pub fn run(text: &str, cfg: &Config) -> Result<String> {
    // Patient here: the alternative to a slow translation is a dictation that
    // never gets typed at all.
    run_into(text, &cfg.translate_language, cfg, ENDPOINT_TIMEOUT)
}

/// The dispatch a dictation and the Settings check share, so what the check
/// proves is exactly what a dictation will do.
fn run_into(text: &str, language: &str, cfg: &Config, timeout: Duration) -> Result<String> {
    match cfg.translate_provider {
        TranslateProvider::Gemini => {
            let key = cfg.translation_gemini_key().ok_or_else(|| {
                anyhow!(
                    "Translating uses Gemini. Add GEMINI_API_KEY to .env, or paste a key under \
                     Settings → Translation."
                )
            })?;
            translate(&key, &cfg.translate_model, language, text, timeout)
        }
        TranslateProvider::Custom => translate_openai(
            &cfg.translate_base_url,
            &cfg.translate_api_key,
            &cfg.translate_custom_model,
            language,
            text,
            timeout,
        ),
    }
}

/// Ask the configured service for a throwaway translation, so a wrong key,
/// model or endpoint is found in Settings rather than halfway through a
/// dictation.
///
/// English in, English out — whatever the user translates *into*. The target
/// has to be pinned for that to hold, and it is the whole point: translating
/// into the user's own language made a working setup report itself broken for
/// anyone not translating into English, which is most of the people who would
/// press this.
///
/// That also makes it the only way to check a Gemini key used for translating
/// and nothing else: the transcription card's check only ever looks at the
/// provider that transcribes.
pub fn check(cfg: &Config) -> Result<String> {
    // Impatient, unlike a dictation: someone is watching this button, and a
    // check that never answers has no error to show and nothing to report.
    let out = run_into("hello", "en", cfg, crate::problem::VERIFY_TIMEOUT)?;
    // What came back is shown rather than judged. Every way this can be wrong —
    // a bad URL, key or model — fails before there is a reply at all, and a
    // model that answers "hi" is a working model.
    Ok(format!("It works — the service returned {out:?}"))
}

/// The `/chat/completions` URL for a base URL.
///
/// What gets pasted varies — `https://api.openai.com/v1`, the same with a
/// trailing slash, or the full path — so the slash comes off and the path is
/// only added when it is not already there.
fn chat_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

/// Translate through any OpenAI-compatible `/chat/completions` endpoint.
pub fn translate_openai(
    base_url: &str,
    key: &str,
    model: &str,
    language: &str,
    text: &str,
    timeout: Duration,
) -> Result<String> {
    let base = base_url.trim();
    if base.is_empty() {
        return Err(anyhow!(
            "No endpoint URL set. Paste one in Settings, under Translation."
        ));
    }
    // Checked here rather than left to ureq, which reports the same thing as a
    // bare URI parse error with no hint of which setting it came from.
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(anyhow!("The translation endpoint URL must start with http:// or https://"));
    }

    let mut req = ureq::post(&chat_url(base))
        .config()
        .timeout_global(Some(timeout))
        // The endpoint's own complaint is in the body; a bare status code says
        // nothing about which of the three settings is wrong.
        .http_status_as_error(false)
        .build()
        .header("Content-Type", "application/json");

    // A local server usually wants no key at all, and an empty `Bearer ` is
    // what makes some of them refuse the request, so the header stays off.
    let key = key.trim();
    if !key.is_empty() {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }

    let resp = req
        .send_json(json!({
            "model": model.trim(),
            "messages": [{
                "role": "user",
                "content": prompt(&crate::state::language_label(language), text),
            }],
            // Deliberately no `temperature`, unlike the Gemini call. The newest
            // OpenAI models reject any value but their default with a 400, and
            // there is no way for the user to work around that from here. The
            // prompt is what keeps the reply to the translation.
        }))
        .map_err(|e| anyhow!("could not reach the endpoint at {base}: {e}"))?;

    let status = resp.status();
    let body = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| anyhow!("reading response: {e}"))?;

    if !status.is_success() {
        return Err(anyhow!(
            "{base} returned HTTP {}: {}",
            status.as_u16(),
            complaint(&body)
        ));
    }

    let v: Value = serde_json::from_slice(&body).map_err(|e| {
        anyhow!("{base} did not return JSON ({e}) — is it an OpenAI-compatible API?")
    })?;

    let out = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if out.is_empty() {
        return Err(anyhow!("the model returned no translation"));
    }
    Ok(out)
}

/// An endpoint's own words about what went wrong. Every OpenAI-compatible
/// server puts the reason at `/error/message`; anything else — an HTML error
/// page from a proxy in the way, say — is at least worth a short snippet.
///
/// Shared with the local transcription provider, which talks to the same kind
/// of endpoint and owes the user the same explanation.
pub(crate) fn complaint(body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        if let Some(m) = v.pointer("/error/message").and_then(Value::as_str) {
            return m.to_string();
        }
    }
    let text = String::from_utf8_lossy(body);
    let snippet: String = text.trim().chars().take(300).collect();
    if snippet.is_empty() {
        "no reason given".to_string()
    } else {
        snippet
    }
}

/// Translate with a named Gemini model into a language code.
pub fn translate(
    key: &str,
    model: &str,
    language: &str,
    text: &str,
    timeout: Duration,
) -> Result<String> {
    let model = model.trim();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
    );

    // Bounded like the custom path. ureq waits forever by default, and a
    // translation that never comes back is worse than one that fails: the
    // failure types the words the user actually said, the hang types nothing.
    let resp = ureq::post(&url)
        .config()
        .timeout_global(Some(timeout))
        .build()
        .header("x-goog-api-key", key)
        .header("Content-Type", "application/json")
        .send_json(json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt(&crate::state::language_label(language), text) }],
            }],
            // A translation has one right answer, and the text is typed out as
            // it comes back, so there is nothing to gain from sampling.
            "generationConfig": { "temperature": 0 },
        }))
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => anyhow!("{model} is not a model this key can use"),
            ureq::Error::StatusCode(code) => anyhow!("Gemini returned HTTP {code}"),
            other => anyhow!("could not reach Gemini: {other}"),
        })?;

    let body = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| anyhow!("reading response: {e}"))?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| anyhow!("unexpected response: {e}"))?;

    // A reply can be split across parts, and carries a thought signature beside
    // the text: only the text is the translation.
    let out: String = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let out = out.trim().to_string();
    if out.is_empty() {
        // Typing nothing would look like the dictation was lost; say so instead.
        return Err(anyhow!("the model returned no translation"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_asks_for_nothing_but_the_translation() {
        let p = prompt("English", "hello\nworld");
        assert!(p.contains("into English"));
        assert!(p.contains("Output only the translation"));
        // Models otherwise break lines at sentence ends, which is visible in
        // whatever gets typed.
        assert!(p.contains("neither more nor fewer"));
        // The transcript is passed through untouched, newlines and all.
        assert!(p.ends_with("hello\nworld"));
    }

    #[test]
    fn language_codes_are_named_for_the_model() {
        // What the settings hold is a code; what a model reads best is a name.
        assert_eq!(crate::state::language_label("fa"), "Persian");
        assert_eq!(crate::state::language_label("en"), "English");
        // An unknown code is still something a model can act on.
        assert_eq!(crate::state::language_label("xx"), "XX");
    }

    /// People paste the base URL every way there is, and the path must come out
    /// the same each time — a doubled `/v1/v1` or a missing `/chat/completions`
    /// is a 404 they have no way to read as "you typed it slightly wrong".
    #[test]
    fn a_base_url_becomes_the_chat_endpoint() {
        for paste in [
            "https://api.openai.com/v1",
            "https://api.openai.com/v1/",
            "  https://api.openai.com/v1  ",
            "https://openrouter.ai/api/v1",
            "http://localhost:11434/v1",
        ] {
            assert_eq!(chat_url(paste), format!("{}/chat/completions", paste.trim().trim_end_matches('/')));
        }
        // Pasting the whole path is fine too, and is not appended to twice.
        assert_eq!(
            chat_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn a_broken_endpoint_is_reported_before_anything_is_sent() {
        let mut cfg = Config {
            translate_provider: TranslateProvider::Custom,
            ..Config::default()
        };

        // Nothing configured: the error has to name the missing setting.
        let e = run("hello", &cfg).unwrap_err().to_string();
        assert!(e.contains("endpoint URL"), "unhelpful: {e}");

        // A bare host is a common paste, and ureq alone would report it as an
        // opaque URI parse failure.
        cfg.translate_base_url = "api.openai.com/v1".into();
        let e = run("hello", &cfg).unwrap_err().to_string();
        assert!(e.contains("http://"), "unhelpful: {e}");
    }

    /// The request and the reply are the whole feature, and the exact JSON
    /// paths in each are the part nothing else catches. A stub server on
    /// loopback is the only way to check both without a live endpoint and a key.
    #[test]
    fn a_call_goes_out_and_a_reply_comes_back() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // The client is waiting on the reply rather than closing, so the
            // request is read until it goes quiet.
            sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = sock.read(&mut buf) {
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }

            // What an OpenAI-compatible service sends back, padding and all.
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"  hello there  "}}]}"#;
            let _ = write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            String::from_utf8_lossy(&request).to_lowercase()
        });

        let out = translate_openai(
            &format!("http://127.0.0.1:{port}/v1"),
            "sk-test",
            "gpt-4o-mini",
            "en",
            "سلام",
            Duration::from_secs(10),
        )
        .expect("the stub answered");
        // Only the reply's text, trimmed: anything else is typed out verbatim.
        assert_eq!(out, "hello there");

        let request = server.join().unwrap();
        assert!(request.starts_with("post /v1/chat/completions "), "wrong path: {request}");
        assert!(request.contains("bearer sk-test"), "no key sent: {request}");
        // Not matched on `"model":"..."` — ureq pretty-prints the body, so the
        // spaces around the colon are not ours to rely on.
        assert!(request.contains("gpt-4o-mini"), "no model: {request}");
        // The instruction and the transcript both have to reach the endpoint.
        assert!(request.contains("into english"), "no instruction: {request}");
        assert!(request.contains("سلام"), "no transcript: {request}");
    }

    #[test]
    fn an_endpoints_own_complaint_is_what_gets_shown() {
        // The shape OpenAI, OpenRouter, Groq and the rest all use.
        let body = br#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
        assert_eq!(complaint(body), "Incorrect API key provided");

        // No JSON to read — a proxy's HTML error page still beats "HTTP 502".
        assert_eq!(complaint(b"<html>Bad Gateway</html>"), "<html>Bad Gateway</html>");
        assert_eq!(complaint(b""), "no reason given");
    }

    /// The Settings check must translate into English whatever the user translates
/// *into*, or every model answers correctly and the check calls it a failure.
///
/// This went wrong once by passing the user's own language through, so it is
/// pinned here against a stub rather than left to a live endpoint: the request
/// that goes out is what is asserted on, not the reply.
#[test]
fn the_check_translates_into_english_whatever_the_user_translates_into() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = sock.read(&mut buf) {
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let body = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
        let _ = write!(
            sock,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        String::from_utf8_lossy(&request).to_lowercase()
    });

    let cfg = Config {
        translate_provider: TranslateProvider::Custom,
        translate_base_url: format!("http://127.0.0.1:{port}/v1"),
        translate_custom_model: "test-model".into(),
        // The case that broke: anything that is not English.
        translate_language: "fa".into(),
        ..Config::default()
    };

    let note = check(&cfg).expect("the stub answered");
    println!("{note}");

    let request = server.join().unwrap();
    assert!(request.contains("into english"), "asked for the wrong language: {request}");
    assert!(!request.contains("persian"), "the user's own language leaked in: {request}");
}

/// Ignored by default — needs the network and an OpenAI-compatible endpoint.
    /// Run with:
    ///   `ORRA_TEST_BASE_URL=... ORRA_TEST_API_KEY=... ORRA_TEST_MODEL=... \
    ///    cargo test translate_openai -- --ignored --nocapture`
    #[test]
    #[ignore = "hits a real endpoint; run with --ignored"]
    fn a_real_custom_endpoint_translates() {
        let base = std::env::var("ORRA_TEST_BASE_URL").expect("ORRA_TEST_BASE_URL must be set");
        let key = std::env::var("ORRA_TEST_API_KEY").unwrap_or_default();
        let model = std::env::var("ORRA_TEST_MODEL").expect("ORRA_TEST_MODEL must be set");

        let out = translate_openai(
            &base,
            &key,
            &model,
            "en",
            "سلام، این یک آزمایش است.\nخط دوم.",
            Duration::from_secs(60),
        )
            .expect("translation failed");
        println!("translated: {out:?}");
        assert!(out.to_lowercase().contains("hello") || out.to_lowercase().contains("test"));
        assert!(out.contains('\n'), "the line break was dropped: {out:?}");
    }

    /// Ignored by default — needs the network and a live key. Run with:
    ///   `cargo test translate -- --ignored --nocapture`
    #[test]
    #[ignore = "hits the Gemini API; run with --ignored"]
    fn a_real_translation_comes_back_as_only_the_translation() {
        let cfg = Config::default();
        let key = cfg.translation_gemini_key().expect("GEMINI_API_KEY must be set");

        // Through the same entry point the app uses, so the prompt and the
        // response parsing are both covered.
        let out = translate(
            &key,
            &cfg.translate_model,
            "en",
            "سلام، این یک آزمایش است.\nخط دوم.",
            Duration::from_secs(60),
        )
            .expect("translation failed");
        println!("translated: {out:?}");

        let lower = out.to_lowercase();
        assert!(lower.contains("hello") || lower.contains("test"), "lost the meaning: {out:?}");
        // Two lines in, two lines out: the line break survives the round trip.
        assert!(out.contains('\n'), "the line break was dropped: {out:?}");
        assert!(!out.contains("Translate"), "answered instead of translating: {out:?}");
    }
}
