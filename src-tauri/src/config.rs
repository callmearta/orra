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
    /// The servers people actually run, each with the port and the path it
    /// normally answers on. Naming them separately is what turns "find out what
    /// URL your server is on, and which of the three APIs it speaks" into a
    /// choice from a list; everything about them is still a setting, and the
    /// URL is still editable for the ports that are not the default.
    Ollama,
    Speaches,
    LocalAi,
    WhisperCpp,
    /// Anything else speaking the same APIs: a URL, a model, and which of the
    /// transports it answers on. This is what the local provider was before the
    /// named ones existed, and a config that says `local` still means it.
    Local,
    /// A model this app downloads and runs itself. See [`crate::engine`].
    Orra,
}

impl Provider {
    /// The environment variable (and `.env` key) this provider's key is read
    /// from, before the copy stored in settings. Empty for everything the user
    /// hosts themselves: there is no convention for naming such a variable, and
    /// most of these servers want no key at all.
    pub fn env_var(self) -> &'static str {
        match self {
            Provider::Deepgram => "DEEPGRAM_API_KEY",
            Provider::AssemblyAi => "ASSEMBLYAI_API_KEY",
            Provider::Gemini => "GEMINI_API_KEY",
            _ => "",
        }
    }

    /// How the provider is named to the user, in errors and settings.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Deepgram => "Deepgram",
            Provider::AssemblyAi => "AssemblyAI",
            Provider::Gemini => "Gemini",
            Provider::Ollama => "Ollama",
            Provider::Speaches => "Speaches",
            Provider::LocalAi => "LocalAI",
            Provider::WhisperCpp => "whisper.cpp",
            Provider::Local => "Custom endpoint",
            Provider::Orra => "Orra — open-source models",
        }
    }

    /// Whether this is one of the servers the user hosts, and so shares the
    /// `local_*` settings and the code in [`crate::local`].
    pub fn is_self_hosted(self) -> bool {
        !matches!(self, Provider::Deepgram | Provider::AssemblyAi | Provider::Gemini)
    }

    /// Where this server normally answers, when it has a usual port.
    ///
    /// `None` for the two that cannot be guessed at: a custom endpoint is
    /// whatever the user pasted, and the model Orra runs itself is on a port
    /// this app picked.
    pub fn preset_url(self) -> Option<&'static str> {
        match self {
            Provider::Ollama => Some("http://localhost:11434/v1"),
            Provider::Speaches => Some("http://localhost:8000/v1"),
            Provider::LocalAi => Some("http://localhost:8080/v1"),
            // Not an OpenAI path: whisper.cpp's own server has only this one,
            // and it is why the endpoint setting is editable at all.
            Provider::WhisperCpp => Some("http://localhost:8080/inference"),
            _ => None,
        }
    }

    /// Whether the server can be asked what models it has.
    ///
    /// False for the two that keep the model out of the request: whisper.cpp's
    /// own endpoint transcribes with the model it was started with, and the
    /// engine Orra runs is told its model at startup. Offering a model list for
    /// them means offering a button that can only fail.
    pub fn has_model_list(self) -> bool {
        !matches!(self, Provider::WhisperCpp | Provider::Orra)
    }

    /// Whether the transport is the user's to choose.
    ///
    /// The engine Orra starts is a whisper.cpp server, so it is HTTP by
    /// definition; the rest are whichever API their server happens to speak.
    pub fn has_transport_choice(self) -> bool {
        self.is_self_hosted() && self != Provider::Orra
    }

    /// Whether this provider takes a language at all.
    ///
    /// Everything but Gemini, which detects the language itself and takes no
    /// code — including the servers the user runs, where the code is a hint
    /// that mostly saves the model the guesswork.
    pub fn has_language(self) -> bool {
        self != Provider::Gemini
    }

    /// Which field of [`Config`] holds this provider's key.
    pub fn key_field(self) -> &'static str {
        match self {
            Provider::Deepgram => "api_key",
            Provider::AssemblyAi => "assemblyai_key",
            Provider::Gemini => "gemini_key",
            _ => "local_key",
        }
    }

    /// Every provider, in the order the settings list them.
    ///
    /// The settings screen draws itself from this rather than from a list of
    /// its own: which of these have a URL to preset, a model list to fetch or a
    /// transport to choose is a fact about the provider, and a second copy of
    /// it in the interface is a second thing to keep in step.
    pub fn all() -> Vec<Provider> {
        vec![
            Provider::Deepgram,
            Provider::AssemblyAi,
            Provider::Gemini,
            Provider::Ollama,
            Provider::Speaches,
            Provider::LocalAi,
            Provider::WhisperCpp,
            Provider::Local,
            Provider::Orra,
        ]
    }
}

/// One provider, as the settings screen needs to know it.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub value: Provider,
    pub label: &'static str,
    pub self_hosted: bool,
    /// The `Config` field holding its key, so the interface does not have to
    /// keep its own copy of that mapping.
    pub key_field: &'static str,
    pub env_var: &'static str,
    /// Where it normally answers, when that can be known. The field is still
    /// the user's to edit — this is what it starts as.
    pub preset_url: Option<&'static str>,
    pub has_model_list: bool,
    pub has_transport_choice: bool,
    /// Whether the dictation language applies, and the switch key with it.
    pub has_language: bool,
}

impl From<Provider> for ProviderInfo {
    fn from(p: Provider) -> Self {
        Self {
            value: p,
            label: p.label(),
            self_hosted: p.is_self_hosted(),
            key_field: p.key_field(),
            env_var: p.env_var(),
            preset_url: p.preset_url(),
            has_model_list: p.has_model_list(),
            has_transport_choice: p.has_transport_choice(),
            has_language: p.has_language(),
        }
    }
}

/// How the local provider talks to its server.
///
/// Three shapes rather than one because no single one is spoken by everything
/// worth pointing at, and they are not interchangeable in what the user gets
/// back: only a WebSocket can carry audio up as it is spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LocalTransport {
    /// `POST {base}/audio/transcriptions`, multipart, the OpenAI shape — which
    /// is also what Ollama, LocalAI, Speaches, vLLM and LM Studio answer. The
    /// transcript arrives when the recording ends.
    #[default]
    Http,
    /// The same request with `stream=true`: the server sends the transcript as
    /// it decodes it. Still nothing until the key is released — the audio only
    /// exists then — but the words arrive as they are decided rather than in
    /// one lump at the end.
    Sse,
    /// The OpenAI Realtime socket, which takes audio continuously. This is the
    /// only transport that shows words in the overlay while they are spoken.
    WebSocket,
}

/// What translates. Separate from [`Provider`] because the two halves of a
/// dictation are independent: transcribing with Deepgram and translating with
/// something else is the normal case, not the odd one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranslateProvider {
    /// Gemini, using the same key the app already resolves for transcription.
    #[default]
    Gemini,
    /// Anything that speaks the OpenAI `/chat/completions` shape: OpenAI itself,
    /// OpenRouter, Groq, a local Ollama or LM Studio.
    Custom,
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
    pub local_key: String,
    pub stt_model: String,
    /// Where the local provider's server is. Not validated here: the whole
    /// point is to point it at something this app has never heard of.
    pub local_base_url: String,
    /// The model to ask that server for. Free text, and empty until a server
    /// has been pointed at and asked what it has.
    pub local_model: String,
    /// Which of the three shapes that server speaks. See [`LocalTransport`].
    pub local_transport: LocalTransport,
    /// The model this app runs on this machine itself, by name from
    /// [`crate::engine::MODELS`]. Empty means the user runs their own server —
    /// which is also how a launch knows whether there is an engine to start.
    pub local_engine_model: String,
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
    /// Which service does the translating.
    pub translate_provider: TranslateProvider,
    /// The Gemini model that does the translating, when Gemini is the provider.
    /// Gemini is the default because it is already configured for the other half
    /// of the app and takes a plain instruction.
    pub translate_model: String,
    /// Base URL of an OpenAI-compatible endpoint, e.g. `https://api.openai.com/v1`.
    /// `/chat/completions` is appended to it.
    pub translate_base_url: String,
    /// The model to ask that endpoint for. Free text: there is no list to offer
    /// for a service we have never heard of.
    pub translate_custom_model: String,
    /// Key for that endpoint. One field per provider, like the transcription
    /// keys, so switching back and forth does not mean re-pasting anything.
    pub translate_api_key: String,
    /// Key for translating with Gemini when it should differ from the one that
    /// transcribes. Empty falls back to the transcription Gemini key, so a
    /// single key pasted once still covers both uses.
    pub translate_gemini_key: String,

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

/// The default push-to-talk key.
///
/// macOS reserves the Command+Option chords for itself — ⌘⌥D toggles the Dock —
/// and a hotkey the system eats is one that never reaches the app, so macOS
/// gets Control+Option, which nothing else claims. Everywhere else keeps the
/// Super+Alt binds the README and the Hyprland blocks use.
pub fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") { "CTRL + ALT + D" } else { "SUPER + ALT + D" }
}

/// The default translate key. See [`default_hotkey`].
pub fn default_translate_hotkey() -> &'static str {
    if cfg!(target_os = "macos") { "CTRL + ALT + T" } else { "SUPER + ALT + T" }
}

/// The default language-cycle key. See [`default_hotkey`].
pub fn default_language_hotkey() -> &'static str {
    if cfg!(target_os = "macos") { "CTRL + ALT + L" } else { "SUPER + ALT + L" }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey().into(),
            mode: Mode::Hold,
            provider: Provider::default(),
            api_key: String::new(),
            assemblyai_key: String::new(),
            gemini_key: String::new(),
            local_key: String::new(),
            stt_model: "nova-3".into(),
            // Empty until someone points at a server: there is no port or path
            // this app could guess at, and guessing wrong is worse than the
            // empty field saying it has not been set up yet.
            local_base_url: String::new(),
            local_model: String::new(),
            local_transport: LocalTransport::default(),
            local_engine_model: String::new(),
            // Multilingual by default: it handles English at least as well as
            // pinning `en`, and follows a switch into the other languages it
            // covers without a settings trip.
            language: crate::deepgram::MULTILINGUAL.into(),
            language_cycle: vec!["multi".into(), "en".into(), "fa".into()],
            language_hotkey: default_language_hotkey().into(),
            mic: String::new(),
            smart_format: true,
            dictionary: Vec::new(),
            replacements: Vec::new(),
            translate_hotkey: default_translate_hotkey().into(),
            translate_language: "en".into(),
            translate_provider: TranslateProvider::default(),
            // Verified working and far less contended than the newest flash.
            translate_model: "gemini-3.5-flash".into(),
            // Empty until someone picks the custom provider: there is no
            // sensible default endpoint or model to guess at.
            translate_base_url: String::new(),
            translate_custom_model: String::new(),
            translate_api_key: String::new(),
            translate_gemini_key: String::new(),
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

/// Where downloaded things live — the engine binary and the model weights.
///
/// Data rather than config: these are megabytes the app fetched and can fetch
/// again, not preferences anyone typed. A `~/.config` that is synced or backed
/// up should not have 1.6 GB of model in it.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("orra"),
        _ => home().join(".local/share/orra"),
    }
}

/// Where the Hyprland config lives, if it is a Lua config.
pub fn hypr_lua_dir() -> PathBuf {
    home().join(".config/hypr/config")
}

pub fn is_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// The Flatpak app id when running inside one, `None` otherwise.
///
/// Not a curiosity: a sandbox's `orra-ctl` sits at `/app/bin/orra-ctl`, a path
/// that does not exist on the host, so a compositor bind pointing at it would
/// fail every time. With this the bind goes through `flatpak run` instead,
/// which re-enters the sandbox and finds both the binary and the config — the
/// port and token live in the sandbox's own copy, and a host-side binary would
/// read the wrong one and be turned away.
pub fn flatpak_id() -> Option<String> {
    std::env::var("FLATPAK_ID").ok().filter(|id| !id.is_empty())
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

        let needed_token = cfg.token.is_empty();
        if needed_token {
            cfg.token = random_token();
        }
        #[cfg(target_os = "macos")]
        let moved_keys = cfg.migrate_macos_hotkeys();
        #[cfg(not(target_os = "macos"))]
        let moved_keys = false;

        // Written once for either reason: a first run needs its token on disk,
        // and a moved key should not be recomputed on every launch.
        if needed_token || moved_keys {
            let _ = cfg.save();
        }
        cfg
    }

    /// Move the macOS defaults that predate this build off the Command+Option
    /// chords, which the system reserves — ⌘⌥D toggles the Dock, so a dictate
    /// key that was never delivered. Only the exact old default is touched: a
    /// key the user typed is left alone, even if it uses Command+Option.
    /// Returns whether anything changed.
    #[cfg(target_os = "macos")]
    fn migrate_macos_hotkeys(&mut self) -> bool {
        let mut moved = false;
        if self.hotkey.trim() == "SUPER + ALT + D" {
            self.hotkey = default_hotkey().to_string();
            moved = true;
        }
        if self.translate_hotkey.trim() == "SUPER + ALT + T" {
            self.translate_hotkey = default_translate_hotkey().to_string();
            moved = true;
        }
        if self.language_hotkey.trim() == "SUPER + ALT + L" {
            self.language_hotkey = default_language_hotkey().to_string();
            moved = true;
        }
        moved
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
        // The local provider has no environment variable to read — there is no
        // convention for naming one — so it is answered from settings alone.
        if let Some(k) = Self::env_or_dotenv(provider.env_var()) {
            return Some(k);
        }
        let stored = match provider {
            Provider::Deepgram => &self.api_key,
            Provider::AssemblyAi => &self.assemblyai_key,
            Provider::Gemini => &self.gemini_key,
            // One field for all of them: they are the same setting — the key
            // for whichever server is on the other end — and switching between
            // two of them should not mean pasting it again.
            _ => &self.local_key,
        };
        non_empty(stored)
    }

    /// The key a Gemini translation uses.
    ///
    /// The environment and a `.env` win here as everywhere else; then the key
    /// set for translating; then the one set for transcribing, so a single
    /// Gemini key pasted once still covers both. Translation is the one place a
    /// Gemini key is wanted while Gemini is *not* transcribing, which is why it
    /// reads a field of its own rather than `gemini_key` when it can.
    pub fn translation_gemini_key(&self) -> Option<String> {
        if let Some(k) = Self::env_or_dotenv(Provider::Gemini.env_var()) {
            return Some(k);
        }
        self.stored_translation_gemini_key()
    }

    /// The stored keys for a Gemini translation, without the environment: the
    /// one set for translating, else the one set for transcribing.
    fn stored_translation_gemini_key(&self) -> Option<String> {
        non_empty(&self.translate_gemini_key).or_else(|| non_empty(&self.gemini_key))
    }

    /// `var` in the environment, then in a nearby `.env`.
    ///
    /// An empty var is skipped rather than looked up, which would match a stray
    /// `=value` line in a `.env`.
    fn env_or_dotenv(var: &str) -> Option<String> {
        if var.is_empty() {
            return None;
        }
        if let Ok(k) = std::env::var(var) {
            if !k.trim().is_empty() {
                return Some(k.trim().to_string());
            }
        }
        dotenv_candidates().iter().find_map(|p| read_dotenv_key(p, var))
    }
}

/// A trimmed, non-empty value, or `None`.
fn non_empty(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty()).then(|| v.to_string())
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

pub(crate) fn random_token() -> String {
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
        // Same rule for translation: the provider that was there all along is
        // the one an older file keeps using, and it needs nothing new filled in.
        assert_eq!(cfg.translate_provider, TranslateProvider::Gemini);
        assert!(cfg.translate_base_url.is_empty());
        // Which key `key_for` then returns depends on the environment this runs
        // in — environment first, then `.env`, then the stored one — so that is
        // deliberately not asserted here.
    }

    /// The macOS defaults had to move off the Command+Option chords, which the
    /// system reserves — ⌘⌥D toggles the Dock — so a config written before that
    /// has to come along; a key the user chose has to stay put.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_reserved_macos_defaults_are_moved_and_chosen_keys_are_not() {
        let mut cfg = Config::default();
        cfg.hotkey = "SUPER + ALT + D".into();
        cfg.translate_hotkey = "SUPER + ALT + T".into();
        cfg.language_hotkey = "SUPER + ALT + L".into();
        assert!(cfg.migrate_macos_hotkeys());
        assert_eq!(cfg.hotkey, "CTRL + ALT + D");
        assert_eq!(cfg.translate_hotkey, "CTRL + ALT + T");
        assert_eq!(cfg.language_hotkey, "CTRL + ALT + L");

        // A chord the user picked is theirs, even a Command+Option one.
        cfg.hotkey = "SUPER + ALT + X".into();
        assert!(!cfg.migrate_macos_hotkeys());
        assert_eq!(cfg.hotkey, "SUPER + ALT + X");
    }

    /// Translating usually wants a key of its own — Gemini translating while
    /// something else transcribes is the ordinary case — but an empty one has to
    /// fall back to the key transcribing uses, so one paste still covers both.
    /// The environment is deliberately not part of this: that depends on the
    /// machine the tests run on.
    #[test]
    fn a_stored_translation_key_falls_back_to_the_transcription_one() {
        let mut cfg = Config::default();
        cfg.gemini_key = "shared".into();
        assert_eq!(cfg.stored_translation_gemini_key().as_deref(), Some("shared"));

        cfg.translate_gemini_key = "  translate  ".into();
        assert_eq!(cfg.stored_translation_gemini_key().as_deref(), Some("translate"));

        // Whitespace is not a key.
        cfg.gemini_key = "   ".into();
        cfg.translate_gemini_key = String::new();
        assert_eq!(cfg.stored_translation_gemini_key(), None);
    }

    #[test]
    fn each_provider_names_its_own_environment_variable() {        assert_eq!(Provider::Deepgram.env_var(), "DEEPGRAM_API_KEY");
        assert_eq!(Provider::AssemblyAi.env_var(), "ASSEMBLYAI_API_KEY");
        assert_eq!(Provider::Gemini.env_var(), "GEMINI_API_KEY");
        // Nothing the user hosts has an environment variable, which is what
        // makes `key_for` answer those from settings alone.
        for p in [Provider::Ollama, Provider::Speaches, Provider::LocalAi, Provider::WhisperCpp, Provider::Local, Provider::Orra] {
            assert_eq!(p.env_var(), "", "{p:?}");
            assert!(p.is_self_hosted(), "{p:?}");
        }
        assert_eq!(Provider::Local.label(), "Custom endpoint");
        // ...and the names the settings UI sends back round-trip.
        for p in [
            Provider::Deepgram,
            Provider::AssemblyAi,
            Provider::Gemini,
            Provider::Ollama,
            Provider::Speaches,
            Provider::LocalAi,
            Provider::WhisperCpp,
            Provider::Local,
            Provider::Orra,
        ] {
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<Provider>(&json).unwrap(), p);
        }
        // The spelling that was shipped first keeps meaning the same thing, so
        // a settings file written then still loads as the custom endpoint
        // rather than failing to parse and taking every other setting with it.
        assert_eq!(serde_json::from_str::<Provider>("\"local\"").unwrap(), Provider::Local);
    }

    /// The presets exist for their URLs, and the two that have none are the two
    /// that cannot be guessed at.
    #[test]
    fn each_named_server_knows_where_it_normally_answers() {
        assert_eq!(Provider::Ollama.preset_url(), Some("http://localhost:11434/v1"));
        assert_eq!(Provider::Speaches.preset_url(), Some("http://localhost:8000/v1"));
        assert_eq!(Provider::LocalAi.preset_url(), Some("http://localhost:8080/v1"));
        // whisper.cpp's own path, which is not an OpenAI one.
        assert_eq!(Provider::WhisperCpp.preset_url(), Some("http://localhost:8080/inference"));
        assert_eq!(Provider::Local.preset_url(), None);
        assert_eq!(Provider::Orra.preset_url(), None);
    }

    /// What the settings card is allowed to offer. A model list belongs to the
    /// servers that keep the model in the request, and the transport is only a
    /// choice where the server has a say in it.
    #[test]
    fn only_the_servers_with_a_model_list_are_offered_one() {
        assert!(Provider::Ollama.has_model_list());
        assert!(Provider::Speaches.has_model_list());
        assert!(Provider::Local.has_model_list());
        // Transcribes with the model it was started with.
        assert!(!Provider::WhisperCpp.has_model_list());
        // Told its model when Orra starts it.
        assert!(!Provider::Orra.has_model_list());

        assert!(Provider::Ollama.has_transport_choice());
        assert!(Provider::WhisperCpp.has_transport_choice());
        assert!(!Provider::Orra.has_transport_choice());
        assert!(!Provider::Deepgram.has_transport_choice());
    }

    /// The local provider is the one whose key is never in the environment, so
    /// a machine with no `ORRA_*` variable set is the normal case rather than a
    /// misconfiguration.
    #[test]
    fn a_local_key_comes_from_settings_and_is_optional() {
        let cfg = Config { local_key: "  sk-local-test-key-1234  ".into(), ..Config::default() };
        assert_eq!(cfg.key_for(Provider::Local).as_deref(), Some("sk-local-test-key-1234"));

        // Nothing stored, nothing required: a server on loopback usually has no
        // auth at all, and asking for a key would make it unusable.
        let bare = Config::default();
        assert_eq!(bare.key_for(Provider::Local), None);
    }

    /// The three transports are spelled the way the settings UI sends them, and
    /// a config written before they existed is an HTTP server rather than a
    /// broken one.
    #[test]
    fn the_local_transport_round_trips_and_defaults_to_http() {
        for (t, json) in [
            (LocalTransport::Http, "\"http\""),
            (LocalTransport::Sse, "\"sse\""),
            (LocalTransport::WebSocket, "\"websocket\""),
        ] {
            assert_eq!(serde_json::to_string(&t).unwrap(), json);
            assert_eq!(serde_json::from_str::<LocalTransport>(json).unwrap(), t);
        }

        let older: Config = serde_json::from_str(r#"{"local_base_url":"http://localhost:8000/v1"}"#)
            .expect("a config from before transports still parses");
        assert_eq!(older.local_transport, LocalTransport::Http);
        assert!(older.local_model.is_empty());
        assert!(older.local_key.is_empty());
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
