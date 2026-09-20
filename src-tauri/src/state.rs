//! Shared application state, the dictation lifecycle, and history.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::{self, Capture};
use crate::config::{self, Config};
use crate::inject;
use crate::deepgram;
use crate::polish::{self, Action};
use crate::stt::{self, SttSession};

/// Keep the history file from growing without bound.
///
/// The Insights page aggregates over this file, so the cap is also the point at
/// which "all time" stops being true. At 500 a heavy user would roll past it in
/// weeks and their totals would start shrinking instead of growing; the file is
/// one JSON object per line, so the larger cap costs a few MB at most.
const HISTORY_CAP: usize = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub text: String,
    /// Unix milliseconds.
    pub at: u64,
    pub app: String,
    pub words: usize,
    /// How long the user spoke, in milliseconds. `None` for entries recorded
    /// before this was tracked and for sessions that ended without a stop, so
    /// every reader has to cope with its absence — and `#[serde(default)]` is
    /// what lets those older lines keep parsing at all. Without it, adding a
    /// field here would make `load_history` discard every existing line.
    #[serde(default)]
    pub ms: Option<u64>,
    /// Replacement rules that fired during this dictation.
    #[serde(default)]
    pub fixes: usize,
    /// Filler words dropped during this dictation.
    #[serde(default)]
    pub fillers: usize,
    /// Mean confidence reported by the provider, 0.0–1.0, over the words of
    /// this dictation. Absent for providers that do not report one.
    #[serde(default)]
    pub confidence: Option<f32>,
    /// What was actually said, when `text` is a translation of it. Absent for
    /// every ordinary dictation, and for entries written before this existed.
    #[serde(default)]
    pub source: Option<String>,
}

/// What a session is for. Translation is the same recording and the same
/// transcript; it only changes what happens to the words at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Type what was said.
    Dictate,
    /// Translate it first, and type that.
    Translate,
}

/// What the stream task observed about a dictation beyond its words.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spoken {
    /// Milliseconds from recording start to the stop, so `0` means unmeasured.
    pub ms: u64,
    pub confidence: Option<f32>,
}

/// A duration of zero means the dictation was never measured, not that it was
/// instantaneous, so it is stored as absent.
fn measured_ms(ms: u64) -> Option<u64> {
    (ms > 0).then_some(ms)
}

/// A live dictation, tagged so a stopping session can tell whether the slot it
/// occupies is still its own.
pub struct Session {
    pub id: u64,
    pub stt: SttSession,
}

pub struct AppState {
    pub cfg: Mutex<Config>,
    pub session: Mutex<Option<Session>>,
    /// The speech-to-text server this app started itself, when the user asked
    /// it to run a model on this machine. Holding it here is what ties its
    /// lifetime to the app's: dropping the slot kills the process.
    pub engine: Mutex<Option<crate::engine::EngineProcess>>,
    pub history: Mutex<Vec<Entry>>,
    /// What the last dictation typed, so "scratch that" knows how much to delete.
    pub last: Mutex<Option<String>>,
    pub speaking: AtomicBool,
    /// Set when the user stops while a start is still connecting, so the start
    /// can notice on arrival instead of leaving a recording running forever.
    pub stop_requested: AtomicBool,
    next_session_id: AtomicU64,
}

impl AppState {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg: Mutex::new(cfg),
            session: Mutex::new(None),
            engine: Mutex::new(None),
            history: Mutex::new(load_history()),
            last: Mutex::new(None),
            speaking: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            next_session_id: AtomicU64::new(1),
        }
    }

    pub fn next_session_id(&self) -> u64 {
        self.next_session_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn is_recording(&self) -> bool {
        self.session.lock().map(|s| s.is_some()).unwrap_or(false)
    }

    /// Stop the active session, if any. Idempotent.
    ///
    /// The flag is set even when there is no session yet: a start that is still
    /// waiting on the socket reads it once it publishes, and stops immediately.
    pub fn stop_session(&self) {
        self.stop_requested.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.session.lock() {
            if let Some(mut s) = guard.take() {
                s.stt.stop();
            }
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// dictation lifecycle
// ---------------------------------------------------------------------------

/// Begin recording. Returns immediately; the transcript arrives via events.
pub async fn start_dictation(app: AppHandle, purpose: Purpose) -> Result<()> {
    let state = app.state::<AppState>();
    if state.is_recording() {
        return Ok(());
    }
    let cfg = state.config();

    // Cleared before anything can await, so a release during the connect below
    // is recorded rather than lost.
    state.stop_requested.store(false, Ordering::SeqCst);
    let id = state.next_session_id();

    // The microphone opens first and the socket is opened behind it, so the
    // level meter and the transcript both start from the keypress rather than
    // from whenever the service's handshake finished. Anything said in between
    // is buffered by the capture channel and sent as soon as the socket is up.
    let capture = Capture::start(&cfg.mic)?;
    let session = stt::start(app.clone(), &cfg, id, capture, purpose)?;
    *state.session.lock().unwrap() = Some(Session { id, stt: session });

    // The key may have been released while the socket was still opening.
    if state.stop_requested.load(Ordering::SeqCst) {
        state.stop_session();
        return Ok(());
    }
    // Rising tone: recording is live and the mic is actually open.
    if cfg.sounds {
        audio::beep(880.0, 70);
    }
    let _ = app.emit(stt::EVT_STATE, "recording");
    Ok(())
}

/// Turn the raw transcript into typed text. Called by the stream task once
/// the provider has flushed.
pub async fn finish_dictation(
    app: &AppHandle,
    id: u64,
    raw: &str,
    cfg: &Config,
    spoken: Spoken,
    purpose: Purpose,
) {
    let state = app.state::<AppState>();
    vacate(&state, id);
    // Falling tone: the mic is closed and the text is on its way.
    if cfg.sounds {
        audio::beep(560.0, 70);
    }

    let processed = polish::process(raw, cfg);

    if processed.action == Action::Scratch {
        let previous = state.last.lock().ok().and_then(|g| g.clone());
        if let Some(prev) = previous {
            let n = prev.chars().count();
            let _ = tokio::task::spawn_blocking(move || inject::press_backspace(n)).await;
        }
        let _ = app.emit(stt::EVT_STATE, "idle");
        return;
    }

    if processed.text.is_empty() {
        let _ = app.emit(stt::EVT_STATE, "idle");
        return;
    }

    // Translation is the last thing before typing, so it sees the cleaned-up
    // transcript rather than the raw one.
    let (typed, source) = match purpose {
        Purpose::Dictate => (processed.text.clone(), None),
        Purpose::Translate => translated(app, &processed.text, cfg).await,
    };

    let mut out = typed.clone();
    if cfg.trailing_space {
        out.push(' ');
    }

    let (text, cfg2, submit) = (out.clone(), cfg.clone(), processed.submit || cfg.auto_submit);
    let delivered = tokio::task::spawn_blocking(move || {
        inject::deliver(&text, &cfg2)?;
        if submit {
            inject::press_enter()?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await;

    match delivered {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            crate::problem::report(app, "Could not type the text", &e);
        }
        Err(e) => {
            crate::problem::report(app, "Could not type the text", format!("insertion task failed: {e}"));
        }
    }

    if let Ok(mut last) = state.last.lock() {
        *last = Some(out.clone());
    }

    // One clock read for both fields: two calls could straddle a millisecond and
    // hand two dictations the same id, which `delete_history` and `reinject`
    // key on.
    let at = now_ms();
    let entry = Entry {
        id: at.to_string(),
        text: typed.clone(),
        source,
        at,
        app: inject::focused_class(),
        // Counted on what was spoken, not on the translation: the words-per-
        // minute figures compare these against the time the microphone was on.
        words: processed.text.split_whitespace().count(),
        // A stop that arrived before any audio did leaves this at zero, which
        // `measured_ms` turns into "not measured".
        ms: measured_ms(spoken.ms),
        fixes: processed.replacements,
        fillers: processed.fillers,
        confidence: spoken.confidence,
    };
    push_history(app, entry);
    let _ = app.emit(stt::EVT_STATE, "idle");
}

/// The translation, or the words themselves when it could not be had: a
/// dictation that fails to translate is still a dictation, and losing it would
/// be the worse outcome.
async fn translated(app: &AppHandle, text: &str, cfg: &Config) -> (String, Option<String>) {
    let (owned, cfg) = (text.to_string(), cfg.clone());
    let done = tokio::task::spawn_blocking(move || crate::translate::run(&owned, &cfg)).await;

    match done {
        Ok(Ok(out)) => (out, Some(text.to_string())),
        Ok(Err(e)) => {
            crate::problem::report(app, "Could not translate", &e);
            (text.to_string(), None)
        }
        Err(e) => {
            crate::problem::report(app, "Could not translate", format!("translation task failed: {e}"));
            (text.to_string(), None)
        }
    }
}

/// End a dictation that never reached Deepgram, so the app does not sit in a
/// recording state that nothing will ever finish. The reason is reported
/// separately, as an error event.
pub fn abandon_dictation(app: &AppHandle, id: u64) {
    vacate(&app.state::<AppState>(), id);
    let _ = app.emit(stt::EVT_STATE, "idle");
}

/// Give up the session slot, unless a newer dictation has already claimed it.
/// Clearing that one instead would leave it unstoppable.
fn vacate(state: &AppState, id: u64) {
    if let Ok(mut guard) = state.session.lock() {
        if guard.as_ref().map(|s| s.id) == Some(id) {
            *guard = None;
        }
    }
}

// ---------------------------------------------------------------------------
// dictation language
// ---------------------------------------------------------------------------

/// Human label for a language code, for the overlay and the settings screen.
pub fn language_label(code: &str) -> String {
    match code {
        "multi" => "Multilingual".to_string(),
        "en" => "English".to_string(),
        "fa" => "Persian".to_string(),
        "ar" => "Arabic".to_string(),
        "he" => "Hebrew".to_string(),
        "tr" => "Turkish".to_string(),
        "ur" => "Urdu".to_string(),
        "ru" => "Russian".to_string(),
        "de" => "German".to_string(),
        "fr" => "French".to_string(),
        "es" => "Spanish".to_string(),
        other => other.to_uppercase(),
    }
}

/// Step to the next language in the configured cycle and persist it.
///
/// Deepgram only auto-detects on the batch endpoint, so on a live stream the
/// language has to be chosen up front; this makes changing it a keystroke
/// instead of a trip through settings.
pub fn cycle_language(app: &AppHandle, backwards: bool) -> Result<String> {
    let state = app.state::<AppState>();
    let (cycle, current) = {
        let cfg = state.cfg.lock().map_err(|_| anyhow!("config lock poisoned"))?;
        (cfg.language_cycle.clone(), cfg.language.clone())
    };

    let usable: Vec<String> = cycle.into_iter().filter(|c| !c.trim().is_empty()).collect();
    if usable.is_empty() {
        return Err(anyhow!("no languages configured to switch between"));
    }

    let step = if backwards { usable.len() - 1 } else { 1 };
    let at = usable.iter().position(|c| *c == current).unwrap_or(0);
    let next = usable[(at + step) % usable.len()].clone();

    set_language(app, &next)?;
    Ok(next)
}

/// Switch to a specific language code and persist it.
pub fn set_language(app: &AppHandle, code: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let sounds = {
        let mut cfg = state.cfg.lock().map_err(|_| anyhow!("config lock poisoned"))?;
        if cfg.language == code {
            return Ok(());
        }
        cfg.language = code.to_string();
        let snapshot = cfg.clone();
        drop(cfg);
        snapshot.save().map_err(|e| anyhow!("could not save settings: {e}"))?;
        snapshot.sounds
    };

    // A dictation already in flight keeps the language it started with, so it is
    // cleaner to stop it than to let the user think the switch applied.
    state.stop_session();

    if sounds {
        audio::beep(1_180.0, 55);
    }
    let _ = app.emit(
        stt::EVT_LANGUAGE,
        serde_json::json!({ "code": code, "label": language_label(code) }),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// text to speech
// ---------------------------------------------------------------------------

/// Speak `text` through the default output. Blocking work runs off-thread.
pub fn speak(app: &AppHandle, text: String) -> Result<()> {
    let state = app.state::<AppState>();
    let cfg = state.config();
    if !cfg.tts_enabled {
        return Err(anyhow!("Read-aloud is turned off in Settings"));
    }
    // Read-aloud is Deepgram's voices, whichever provider is transcribing, so
    // it wants the Deepgram key rather than the configured one.
    let key = cfg.key_for(config::Provider::Deepgram).ok_or_else(|| {
        anyhow!("Read-aloud uses Deepgram's voices. Add DEEPGRAM_API_KEY to .env or paste it in Settings.")
    })?;

    let chunks = deepgram::chunk_for_speech(&text);
    if chunks.is_empty() {
        return Err(anyhow!("Nothing to read"));
    }

    state.speaking.store(true, Ordering::Relaxed);
    audio::stop_speaking(); // cut any previous utterance

    let handle = app.clone();
    std::thread::spawn(move || {
        for chunk in chunks {
            if !handle.state::<AppState>().speaking.load(Ordering::Relaxed) {
                break;
            }
            match deepgram::speak_chunk(&key, &cfg.tts_model, &chunk) {
                Ok(wav) => {
                    if let Err(e) = audio::play_wav(wav) {
                        crate::problem::report(&handle, "Could not play the speech", e);
                        break;
                    }
                }
                Err(e) => {
                    crate::problem::report(&handle, "Could not read that aloud", e);
                    break;
                }
            }
        }
        handle.state::<AppState>().speaking.store(false, Ordering::Relaxed);
        let _ = handle.emit(stt::EVT_STATE, "idle");
    });
    Ok(())
}

pub fn stop_speaking(app: &AppHandle) {
    app.state::<AppState>().speaking.store(false, Ordering::Relaxed);
    audio::stop_speaking();
}

// ---------------------------------------------------------------------------
// history
// ---------------------------------------------------------------------------

fn load_history() -> Vec<Entry> {
    let Ok(body) = std::fs::read_to_string(config::history_path()) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = body
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if out.len() > HISTORY_CAP {
        out.drain(..out.len() - HISTORY_CAP);
    }
    out
}

fn push_history(app: &AppHandle, entry: Entry) {
    let state = app.state::<AppState>();
    if let Ok(mut h) = state.history.lock() {
        h.push(entry.clone());
        if h.len() > HISTORY_CAP {
            let excess = h.len() - HISTORY_CAP;
            h.drain(..excess);
        }
    }
    // Append-only, one JSON object per line: a crash can lose the last line but
    // never corrupt what came before, and rewriting the file is never needed.
    let path = config::history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let (Ok(line), Ok(mut f)) = (
        serde_json::to_string(&entry),
        std::fs::OpenOptions::new().create(true).append(true).open(&path),
    ) {
        let _ = writeln!(f, "{line}");
    }
    let _ = app.emit(stt::EVT_HISTORY, ());
}

pub fn clear_history(app: &AppHandle) {
    if let Ok(mut h) = app.state::<AppState>().history.lock() {
        h.clear();
    }
    let _ = std::fs::remove_file(config::history_path());
    let _ = app.emit(stt::EVT_HISTORY, ());
}

pub fn delete_history(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    if let Ok(mut h) = state.history.lock() {
        h.retain(|e| e.id != id);
        // Rewrite rather than append: deletion is rare, and the file is small
        // enough that a full rewrite is simpler than a tombstone scheme.
        let body: String = h
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .map(|l| l + "\n")
            .collect();
        let _ = std::fs::write(config::history_path(), body);
    }
    let _ = app.emit(stt::EVT_HISTORY, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_round_trips_through_jsonl() {
        let e = Entry {
            id: "1".into(),
            text: "hello\nworld".into(),
            at: 42,
            app: "kitty".into(),
            words: 2,
            ms: Some(4_200),
            fixes: 3,
            fillers: 1,
            confidence: Some(0.97),
            source: Some("سلام".into()),
        };
        let line = serde_json::to_string(&e).unwrap();
        // Embedded newlines must not break the one-object-per-line format.
        assert!(!line.contains('\n'));
        let back: Entry = serde_json::from_str(&line).unwrap();
        assert_eq!(back.text, "hello\nworld");
        assert_eq!(back.ms, Some(4_200));
        assert_eq!(back.fixes, 3);
        assert_eq!(back.fillers, 1);
        assert_eq!(back.confidence, Some(0.97));
        // The words that were spoken before a translation are kept with it.
        assert_eq!(back.source.as_deref(), Some("سلام"));
    }

    #[test]
    fn a_line_written_before_the_new_fields_still_parses() {
        // `load_history` reads each line with `filter_map(..ok())`, so a line
        // that fails to parse is dropped without a word. This is the guard that
        // the four `#[serde(default)]` attributes exist for: if any of them is
        // removed, this test fails instead of the user's history disappearing.
        let legacy = r#"{"id":"1789649584586","text":"hello","at":1789649584586,"app":"kitty","words":1}"#;
        let e: Entry = serde_json::from_str(legacy).expect("legacy line must still parse");
        assert_eq!(e.text, "hello");
        assert_eq!(e.ms, None);
        assert_eq!(e.fixes, 0);
        assert_eq!(e.fillers, 0);
        assert_eq!(e.confidence, None);
        assert_eq!(e.source, None);
    }

    #[test]
    fn a_zero_duration_means_unmeasured() {
        // The stream task reports 0 ms when a stop lands before any audio did.
        // Keeping that as `Some(0)` would drag the words-per-minute average to
        // nothing, so the convention is that 0 becomes absent.
        assert_eq!(measured_ms(0), None);
        assert_eq!(measured_ms(1), Some(1));
    }
}
