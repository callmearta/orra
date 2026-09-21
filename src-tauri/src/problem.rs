//! Turning a failure into something worth showing the user.
//!
//! Every failure the app reports goes through here, for two reasons. The user
//! is told what *kind* of failure it was — reach the service, check the key,
//! start the local endpoint — before being handed a line of raw error, because
//! those are what decide whether the problem is theirs to fix. And, the reason
//! this is a module rather than a format string at each call site: an API key
//! cannot ride out inside a message we invite people to paste into a bug report.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{Config, Provider};
use crate::state::AppState;

/// How long a Settings check waits on a service before calling it a failure.
///
/// Without one, a request that connects and then stalls leaves the button
/// reading "Checking…" forever — the one outcome a check must not have, since
/// there is then no error to read and nothing to report.
pub const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// What kind of failure this was.
///
/// A best-effort reading of the error text, not a parse — the messages come
/// from whichever library failed. It is allowed to be wrong: [`Problem::detail`]
/// is always exact, and the kind is only there to point at the right place to
/// look first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The service could not be reached at all.
    Network,
    /// It was reached, and refused the key.
    Auth,
    /// It was reached, and refused the work: rate limit, quota, billing.
    Quota,
    /// It was reached, and did not accept the request: a model or endpoint that
    /// is not there.
    Request,
    /// Nothing was sent, because the app has not been given what it needs yet:
    /// no key, or no endpoint URL. The likeliest first failure there is, and
    /// the only one the user can fix before anything leaves the machine.
    Config,
    /// This machine: the microphone, the clipboard, a tool that is missing.
    Local,
    Unknown,
}

impl Kind {
    /// The headline. Named the way a user would name it, because "check your
    /// connection" and "check your key" are the two things worth knowing
    /// before reading any of the error itself.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Network => "Could not reach the service",
            Kind::Auth => "The API key was refused",
            Kind::Quota => "Rate limit or quota reached",
            Kind::Request => "The service refused the request",
            Kind::Config => "Not set up yet",
            Kind::Local => "Something on this machine failed",
            Kind::Unknown => "Something went wrong",
        }
    }

    /// What to do about it. Kept to things the user can actually act on.
    pub fn advice(self) -> &'static str {
        match self {
            Kind::Network => {
                "Check this machine's connection and try again. An endpoint running locally \
                 has to be started first."
            }
            Kind::Auth => {
                "Check the key in Settings, and that the account it belongs to is active."
            }
            Kind::Quota => {
                "Wait a moment and try again, or check the plan and billing on that account."
            }
            Kind::Request => {
                "Check the model or endpoint name in Settings against what that service offers."
            }
            Kind::Config => {
                "Fill in what is missing under Settings — the detail says which one, and \
                 nothing was sent until it is there."
            }
            Kind::Local => {
                "The detail below names what failed — usually the microphone, the clipboard, \
                 or a tool that is not installed."
            }
            Kind::Unknown => "The detail below is the whole error, and it is what to send us.",
        }
    }

    /// Read a kind off an error message.
    ///
    /// Ordered so the more specific reading wins. A refusal that quotes a
    /// status code is about the key or the account, not about the network that
    /// carried it, and a broken pipe to the clipboard tool is this machine
    /// rather than the internet.
    pub fn of(text: &str) -> Kind {
        let t = text.to_ascii_lowercase();
        let any = |needles: &[&str]| needles.iter().any(|n| t.contains(n));

        if any(&[
            "401",
            "403",
            "rejected that key",
            "invalid api key",
            "incorrect api key",
            "unauthorized",
            "api key is not a valid",
        ]) {
            return Kind::Auth;
        }
        if any(&["429", "rate limit", "quota", "insufficient", "402", "billing", "out of credit"]) {
            return Kind::Quota;
        }
        // Before the network patterns, and before Local: this one is not a
        // failure at all so much as a setting nobody has filled in, and saying
        // so is more use than any diagnosis of where it failed.
        if any(&[
            "no endpoint url",
            "api key yet",
            "paste it in settings",
            ".env",
            "no model name",
        ]) {
            return Kind::Config;
        }
        if any(&[
            "microphone",
            "input device",
            "clipboard",
            "injection tool",
            "broken pipe",
            "keystrokes",
            "could not insert",
            "insertion task",
            "playback",
            "audio",
            "could not save",
            "autostart",
            "platform",
            "no such file",
            "is not implemented",
        ]) {
            return Kind::Local;
        }
        if any(&[
            "could not reach",
            "connection refused",
            "did not answer",
            "never started",
            "never answered",
            "timed out",
            "timeout",
            "dns",
            "network",
            "stream error",
            "connection reset",
            "disconnected",
            "unreachable",
        ]) {
            return Kind::Network;
        }
        if any(&[
            "returned http",
            "404",
            "not a model this key can use",
            "did not return json",
            "openai-compatible",
            "unexpected response",
        ]) {
            return Kind::Request;
        }
        Kind::Unknown
    }
}

/// A failure, in the shape the interface shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    pub kind: Kind,
    /// The headline: what kind of failure this was.
    pub title: String,
    /// What was being attempted, in the user's terms.
    pub summary: String,
    /// What to do about it.
    pub advice: String,
    /// The failure itself, as it came back, with any key taken out.
    pub detail: String,
    /// Everything above with the version and the platform around it, ready to
    /// paste into a report. Built here rather than in the interface because the
    /// redaction has to happen before the text leaves this process.
    pub log: String,
}

impl Problem {
    /// `summary` is what was being attempted — every call site knows that far
    /// better than any classifier could, so it is passed in rather than guessed.
    pub fn new(summary: impl Into<String>, err: impl std::fmt::Display, cfg: &Config) -> Self {
        // `{:#}` and not `{}`: on an `anyhow::Error` that prints the whole
        // chain — "could not reach X: connection refused" — where `{}` prints
        // only the outermost context, which on its own does not say why. On a
        // plain string the two are the same thing.
        let detail = redact(&format!("{err:#}"), cfg);
        // The summary goes through it too. Every call site passes a constant
        // today, but "nothing leaves this process unredacted" is the whole
        // promise, and a redaction you have to remember to apply is one that
        // gets forgotten.
        let (kind, summary) = (Kind::of(&detail), redact(summary.into().trim(), cfg));

        let log = format!(
            "Orra {version} ({os})\n\
             {when}\n\
             \n\
             Kind:    {title}\n\
             Problem: {summary}\n\
             Advice:  {advice}\n\
             Detail:  {detail}\n",
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z"),
            title = kind.title(),
            advice = kind.advice(),
        );

        Self {
            kind,
            title: kind.title().to_string(),
            summary,
            advice: kind.advice().to_string(),
            detail,
            log,
        }
    }
}

/// Report a failure to the interface. One line at the call site.
pub fn report(app: &AppHandle, summary: &str, err: impl std::fmt::Display) {
    let cfg = app.state::<AppState>().config();
    let _ = app.emit(crate::stt::EVT_ERROR, Problem::new(summary, err, &cfg));
}

/// Take every key this app is holding out of a message.
///
/// The failure this exists for is real: Gemini's key travels in the URL's query
/// string, so any error that quotes that URL would carry the key into a log the
/// user is invited to send us.
fn redact(text: &str, cfg: &Config) -> String {
    let mut out = text.to_string();
    for secret in secrets(cfg) {
        out = out.replace(&secret, "[redacted]");
    }
    out
}

/// Every secret the app holds, from whichever source it resolves to —
/// environment, a nearby `.env`, or settings — so redaction does not depend on
/// where a key came from.
///
/// Trimmed, because that is how it is sent: a key pasted with a leading space
/// goes on the wire without it, so matching only the stored spelling would miss
/// the one that actually appears in a message. And in both spellings, because
/// Gemini's key is percent-encoded into the URL it is sent in.
fn secrets(cfg: &Config) -> Vec<String> {
    // The local provider's key is in here for the same reason as the rest: it is
    // one field the user could have pasted a real OpenAI or Groq key into, and a
    // request to a server that refuses it quotes it back.
    let mut out: Vec<String> =
        [Provider::Deepgram, Provider::AssemblyAi, Provider::Gemini, Provider::Local]
            .into_iter()
            .filter_map(|p| cfg.key_for(p))
            .collect();
    out.push(cfg.translate_api_key.clone());
    // Gemini's key for translating is separate from the one `key_for` resolves,
    // so a message that quoted it would otherwise go out in the clear.
    out.push(cfg.translate_gemini_key.clone());
    out.push(cfg.token.clone());

    let encoded: Vec<String> = out.iter().map(|s| crate::stt::url_encode(s.trim())).collect();
    for secret in out.iter_mut() {
        *secret = secret.trim().to_string();
    }
    out.extend(encoded);
    // A short "key" would redact half the message it appeared in.
    out.retain(|s| s.len() >= 8);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_named_before_it_is_quoted() {
        // Verbatim from the call sites that produce them, because this is a
        // reading of what those messages say and nothing else.
        for (message, want) in [
            (
                "could not reach the transcription service: IO error: Connection refused (os error 111)",
                Kind::Network,
            ),
            ("the transcription service did not answer within 10s", Kind::Network),
            // The other deadline, which said "no response within 10s" and was
            // read as Unknown. Both are pinned, because they are two different
            // strings in stt.rs and only one of them was tested.
            ("did not answer within 10s", Kind::Network),
            ("stream error: Connection reset without closing handshake", Kind::Network),
            ("Gemini rejected that key (401 Unauthorized)", Kind::Auth),
            (
                "https://api.openai.com/v1 returned HTTP 401: Incorrect API key provided",
                Kind::Auth,
            ),
            ("Deepgram returned HTTP 429", Kind::Quota),
            (
                "https://api.openai.com/v1 returned HTTP 404: model not found",
                Kind::Request,
            ),
            ("Groq did not return JSON — is it an OpenAI-compatible API?", Kind::Request),
            // The two a first run actually hits.
            (
                "No endpoint URL set. Paste one in Settings, under Translation.",
                Kind::Config,
            ),
            (
                "Translating uses Gemini. Add GEMINI_API_KEY to .env or paste it in Settings.",
                Kind::Config,
            ),
            ("no default input device", Kind::Local),
            ("cannot decode speech audio: unsupported sample format", Kind::Local),
            ("the system rejected the synthesized keystrokes", Kind::Local),
            ("unsupported platform", Kind::Local),
            ("no way to send Return on this platform", Kind::Local),
            ("wtype: No such file or directory (os error 2)", Kind::Local),
            ("could not insert text: no clipboard tool found (install wl-clipboard)", Kind::Local),
            ("playback failed: cannot open audio output", Kind::Local),
            ("the model returned no translation", Kind::Unknown),
        ] {
            assert_eq!(Kind::of(message), want, "misread: {message}");
        }
    }

    /// The environment decides which key this test actually holds — that is the
    /// point. Whatever `key_for` resolves to is a value the app is using, and
    /// none of them may survive into a report.
    #[test]
    fn no_key_in_use_survives_into_a_message() {
        let cfg = Config {
            gemini_key: "AIzaSyTestKeyValue123".into(),
            translate_gemini_key: "AIzaSyTranslateOnlyKey456".into(),
            translate_api_key: "sk-custom-test-key-9876".into(),
            token: "deadbeefdeadbeef1234".into(),
            ..Config::default()
        };

        for secret in secrets(&cfg) {
            let p = Problem::new("Could not translate", format!("failed with {secret}"), &cfg);
            assert!(!p.detail.contains(&secret), "leaked into the detail: {}", p.detail);
            assert!(!p.log.contains(&secret), "leaked into the log: {}", p.log);
            assert!(p.detail.contains("[redacted]"), "nothing was redacted: {}", p.detail);
        }

        // The custom endpoint's key has no environment source, so it is always
        // the stored one and always covered.
        assert!(secrets(&cfg).iter().any(|s| s == "sk-custom-test-key-9876"));
        // And the Gemini key used only for translating, which `key_for` never
        // resolves and which therefore has no other way in.
        assert!(secrets(&cfg).iter().any(|s| s == "AIzaSyTranslateOnlyKey456"));
    }

    /// A key pasted with a stray space is sent without it, so redacting only the
    /// stored spelling would leave the one that goes on the wire in the clear —
    /// and the endpoint's own error message is quoted back into the detail.
    #[test]
    fn a_key_is_redacted_in_the_spelling_it_is_sent_in() {
        let cfg = Config {
            translate_api_key: "  sk-padded-test-key-1234  ".into(),
            ..Config::default()
        };

        let p = Problem::new(
            "Could not translate",
            "returned HTTP 401: Authorization: Bearer sk-padded-test-key-1234 was rejected",
            &cfg,
        );
        assert!(!p.log.contains("sk-padded-test-key-1234"), "leaked: {}", p.log);
        assert!(p.log.contains("[redacted]"));
    }

    #[test]
    fn the_whole_chain_is_kept_not_just_the_outermost_context() {
        let err = anyhow::anyhow!("Connection refused (os error 111)")
            .context("could not reach the transcription service");
        let p = Problem::new("Could not start dictating", &err, &Config::default());

        // The outermost context alone does not say why it failed...
        assert!(p.detail.contains("could not reach the transcription service"));
        // ...and the cause alone does not say what was being attempted.
        assert!(p.detail.contains("Connection refused"));
    }

    #[test]
    fn the_log_carries_what_a_report_needs() {
        let p = Problem::new("Could not translate", "connection refused", &Config::default());

        assert_eq!(p.kind, Kind::Network);
        assert_eq!(p.title, "Could not reach the service");
        // Which build, and on what.
        assert!(p.log.contains(env!("CARGO_PKG_VERSION")));
        assert!(p.log.contains(std::env::consts::OS));
        // And the three lines that make it actionable.
        assert!(p.log.contains("Could not reach the service"));
        assert!(p.log.contains("Could not translate"));
        assert!(p.log.contains("connection refused"));
        assert!(p.log.contains(p.advice.as_str()));
    }

    #[test]
    fn a_summary_is_not_left_with_the_whitespace_it_was_written_with() {
        let p = Problem::new("  Could not translate  ", "nope", &Config::default());
        assert_eq!(p.summary, "Could not translate");
    }
}
