//! Settings persistence + path/secret resolution.
//!
//! Everything lives under `$XDG_CONFIG_HOME/orra` (or `~/.config/orra`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Hold the key to record, release to insert.
    #[default]
    Hold,
    /// Tap to start, tap again to stop.
    Toggle,
}

/// Which service transcribes. Each streams a live transcript; they differ in
/// what they cost, what they hear best, and what the transcript looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Deepgram,
    AssemblyAi,
    Gemini,
}

impl Provider {
    /// The environment variable (and `.env` key) this provider's key is read
    /// from, before the copy stored in settings.
    pub fn env_var(self) -> &'static str {
        match self {
            Provider::Deepgram => "DEEPGRAM_API_KEY",
            Provider::AssemblyAi => "ASSEMBLYAI_API_KEY",
            Provider::Gemini => "GEMINI_API_KEY",
        }
    }

    /// How the provider is named to the user, in errors and settings.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Deepgram => "Deepgram",
            Provider::AssemblyAi => "AssemblyAI",
            Provider::Gemini => "Gemini",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Injection {
    /// Put the text on the clipboard, synthesize Ctrl+V, restore the old clipboard.
    #[default]
    ClipboardPaste,
    /// Type the characters directly, never touching the clipboard.
    Type,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ---- trigger ----
    /// Hyprland-style hotkey, e.g. `SUPER + ALT + D`. Written verbatim into keybinds.lua.
    pub hotkey: String,
    pub mode: Mode,

    // ---- speech to text ----
    /// Which service does the transcription.
    pub provider: Provider,
    /// Key for the configured provider, when it is not in the environment or a
    /// nearby `.env`. One field per provider rather than one shared field, so
    /// switching back and forth does not make anyone re-paste a key.
    pub api_key: String,
    pub assemblyai_key: String,
    pub gemini_key: String,
    pub stt_model: String,
    /// A Deepgram language code, or `multi` for nova-3's multilingual mode.
    pub language: String,
    /// The language hotkey steps through this list. Kept short — it is a quick
    /// switch for people who dictate in more than one language, since Deepgram
    /// has no automatic detection on the streaming endpoint.
    pub language_cycle: Vec<String>,
    pub language_hotkey: String,
    /// Substring match against the input device name. Empty = system default.
    pub mic: String,
    pub smart_format: bool,
    /// Deepgram keyterms built from the dictionary tab.
    pub dictionary: Vec<String>,
    pub replacements: Vec<Rule>,

    // ---- translation ----
    /// Hold this key and the transcript is translated before it is typed.
    pub translate_hotkey: String,
    /// The language the translation comes out in, as a code: `en`, `fa`, ...
    pub translate_language: String,
    /// The Gemini model that does the translating. Gemini is the only service
    /// here that both takes a plain instruction and is already configured for
    /// the other half of the app.
    pub translate_model: String,

    // ---- text to speech ----
    pub tts_enabled: bool,
    pub tts_model: String,
    pub tts_autoplay: bool,

    // ---- delivery ----
    pub injection: Injection,
    pub restore_clipboard: bool,
    /// Append a space so back-to-back dictations do not run together.
    pub trailing_space: bool,
    /// Press Enter after inserting.
    pub auto_submit: bool,
    /// Convert spoken "new line", "press enter", "scratch that", ...
    pub voice_commands: bool,
    /// Drop "um", "uh", "hmm" and friends.
    pub remove_fillers: bool,

    // ---- interface ----
    pub hud: bool,
    pub sounds: bool,
    pub launch_at_login: bool,
    /// Dashboard theme. Kept here rather than in webview storage so it follows
    /// the same file as every other preference the app remembers, and survives
    /// a cleared webview profile.
    pub dark_mode: bool,

    // ---- plumbing ----
    /// Local control port used by `orra-ctl` (the Hyprland binds call it).
    pub port: u16,
    /// Shared secret so unrelated local processes cannot make us type.
    pub token: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "SUPER + ALT + D".into(),
            mode: Mode::Hold,
            provider: Provider::default(),
            api_key: String::new(),
            assemblyai_key: String::new(),
            gemini_key: String::new(),
            stt_model: "nova-3".into(),
            // Multilingual by default: it handles English at least as well as
            // pinning `en`, and follows a switch into the other languages it
            // covers without a settings trip.
            language: crate::deepgram::MULTILINGUAL.into(),
            language_cycle: vec!["multi".into(), "en".into(), "fa".into()],
            language_hotkey: "SUPER + ALT + L".into(),
            mic: String::new(),
            smart_format: true,
            dictionary: Vec::new(),
            replacements: Vec::new(),
            translate_hotkey: "SUPER + ALT + T".into(),
            translate_language: "en".into(),
            // Verified working and far less contended than the newest flash.
            translate_model: "gemini-3.5-flash".into(),
            tts_enabled: true,
            tts_model: "aura-2-thalia-en".into(),
            tts_autoplay: false,
            injection: Injection::ClipboardPaste,
            restore_clipboard: true,
            trailing_space: true,
            auto_submit: false,
            voice_commands: true,
            remove_fillers: true,
            hud: true,
            sounds: true,
            launch_at_login: false,
            dark_mode: false,
            port: 47811,
            token: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// paths
// ---------------------------------------------------------------------------

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("orra"),
        _ => home().join(".config/orra"),
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn history_path() -> PathBuf {
    config_dir().join("history.jsonl")
}

/// Where the Hyprland config lives, if it is a Lua config.
pub fn hypr_lua_dir() -> PathBuf {
    home().join(".config/hypr/config")
}

pub fn is_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

// ---------------------------------------------------------------------------
// load / save
// ---------------------------------------------------------------------------

impl Config {
    pub fn load() -> Self {
        let mut cfg: Config = std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if cfg.token.is_empty() {
            cfg.token = random_token();
            let _ = cfg.save();
        }
        cfg
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let body = serde_json::to_string_pretty(self).unwrap_or_default();
        // Write-then-rename so a crash mid-write cannot leave a truncated config.
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, dir.join("config.json"))
    }

    /// The key for whichever provider is configured.
    pub fn api_key(&self) -> Option<String> {
        self.key_for(self.provider)
    }

    /// Environment first, then any nearby `.env`, then the stored override.
    pub fn key_for(&self, provider: Provider) -> Option<String> {
        let var = provider.env_var();
        if let Ok(k) = std::env::var(var) {
            if !k.trim().is_empty() {
                return Some(k.trim().to_string());
            }
        }
        for p in dotenv_candidates() {
            if let Some(k) = read_dotenv_key(&p, var) {
                return Some(k);
            }
        }
        let stored = match provider {
            Provider::Deepgram => &self.api_key,
            Provider::AssemblyAi => &self.assemblyai_key,
            Provider::Gemini => &self.gemini_key,
        };
        let k = stored.trim();
        (!k.is_empty()).then(|| k.to_string())
    }
}

fn dotenv_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.join(".env"));
        // `cargo run` starts in src-tauri/, the repo .env is one level up.
        if let Some(parent) = cwd.parent() {
            v.push(parent.join(".env"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join(".env"));
        }
    }
    v.push(config_dir().join(".env"));
    v
}

/// Minimal `KEY=value` reader — enough for a one-line .env, no dependency needed.
fn read_dotenv_key(path: &std::path::Path, key: &str) -> Option<String> {
    let body = std::fs::read_to_string(path).ok()?;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, val)) = line.split_once('=') else { continue };
        if k.trim() != key {
            continue;
        }
        let val = val.trim().trim_matches('"').trim_matches('\'').trim();
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

fn random_token() -> String {
    // No rand dependency: the nanoseconds clock plus the pid is plenty of
    // entropy for a loopback-only shared secret.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", nanos, std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file written before there were providers still has to load,
    /// and still has to mean Deepgram — it is the one that was there. `load`
    /// falls back to `Config::default` for any field it cannot find, so a
    /// missing `provider` is Deepgram rather than "no provider".
    #[test]
    fn a_settings_file_from_before_providers_still_loads() {
        let old = r#"{"hotkey":"CTRL + ALT + D","mode":"hold","stt_model":"nova-3","api_key":"stored"}"#;
        let cfg: Config = serde_json::from_str(old).expect("an older config must still parse");
        assert_eq!(cfg.provider, Provider::Deepgram);
        assert_eq!(cfg.api_key, "stored");
        assert!(cfg.assemblyai_key.is_empty());
        assert!(cfg.gemini_key.is_empty());
        // Which key `key_for` then returns depends on the environment this runs
        // in — environment first, then `.env`, then the stored one — so that is
        // deliberately not asserted here.
    }

    #[test]
    fn each_provider_names_its_own_environment_variable() {
        assert_eq!(Provider::Deepgram.env_var(), "DEEPGRAM_API_KEY");
        assert_eq!(Provider::AssemblyAi.env_var(), "ASSEMBLYAI_API_KEY");
        assert_eq!(Provider::Gemini.env_var(), "GEMINI_API_KEY");
        // ...and the names the settings UI sends back round-trip.
        for p in [Provider::Deepgram, Provider::AssemblyAi, Provider::Gemini] {
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<Provider>(&json).unwrap(), p);
        }
    }

    #[test]
    fn dotenv_parsing() {
        let p = std::env::temp_dir().join("orra_test.env");
        std::fs::write(&p, "# comment\nDEEPGRAM_API_KEY=\"abc123\"\nOTHER=1\n").unwrap();
        assert_eq!(read_dotenv_key(&p, "DEEPGRAM_API_KEY").as_deref(), Some("abc123"));
        assert_eq!(read_dotenv_key(&p, "MISSING"), None);
        let _ = std::fs::remove_file(&p);

        // A partial config file must still deserialize, filling in defaults.
        let cfg: Config = serde_json::from_str(r#"{"hotkey":"SUPER + X"}"#).unwrap();
        assert_eq!(cfg.hotkey, "SUPER + X");
        assert_eq!(cfg.stt_model, "nova-3");
        assert_eq!(cfg.injection, Injection::ClipboardPaste);
    }
}
