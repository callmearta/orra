//! Turning a finished transcript into another language, with an LLM.
//!
//! This runs after the transcript has been cleaned up and before anything is
//! typed, so what lands in the focused window is the translation rather than
//! the words that were spoken. It is the one place a dictation waits on a
//! second network call, which is why it is a deliberate keystroke of its own
//! rather than something every dictation pays for.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::config::{Config, Provider};

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
    let key = cfg.key_for(Provider::Gemini).ok_or_else(|| {
        anyhow!(
            "Translating uses Gemini. Add GEMINI_API_KEY to .env or paste it in Settings."
        )
    })?;
    translate(&key, &cfg.translate_model, &cfg.translate_language, text)
}

/// Translate with a named Gemini model into a language code.
pub fn translate(key: &str, model: &str, language: &str, text: &str) -> Result<String> {
    let model = model.trim();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
    );

    let resp = ureq::post(&url)
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

    /// Ignored by default — needs the network and a live key. Run with:
    ///   `cargo test translate -- --ignored --nocapture`
    #[test]
    #[ignore = "hits the Gemini API; run with --ignored"]
    fn a_real_translation_comes_back_as_only_the_translation() {
        let cfg = Config::default();
        let key = cfg.key_for(Provider::Gemini).expect("GEMINI_API_KEY must be set");

        // Through the same entry point the app uses, so the prompt and the
        // response parsing are both covered.
        let out = translate(&key, &cfg.translate_model, "en", "سلام، این یک آزمایش است.\nخط دوم.")
            .expect("translation failed");
        println!("translated: {out:?}");

        let lower = out.to_lowercase();
        assert!(lower.contains("hello") || lower.contains("test"), "lost the meaning: {out:?}");
        // Two lines in, two lines out: the line break survives the round trip.
        assert!(out.contains('\n'), "the line break was dropped: {out:?}");
        assert!(!out.contains("Translate"), "answered instead of translating: {out:?}");
    }
}
