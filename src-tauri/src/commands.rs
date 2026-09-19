//! The command surface the settings UI talks to.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, Config, Provider};
use crate::deepgram;
use crate::{assemblyai, gemini};
use crate::hotkeys;
use crate::inject;
use crate::state::{self, AppState, Entry, Purpose};

#[derive(Serialize)]
pub struct Status {
    config: Config,
    recording: bool,
    speaking: bool,
    /// True when the push-to-talk bind is owned by Hyprland rather than the app.
    hyprland: bool,
    /// True when a key for the configured provider could be resolved from any
    /// source — environment, a nearby `.env`, or settings.
    has_key: bool,
    version: String,
}

fn status_of(app: &AppHandle) -> Status {
    let state = app.state::<AppState>();
    let cfg = state.config();
    Status {
        has_key: cfg.api_key().is_some(),
        config: cfg,
        recording: state.is_recording(),
        speaking: state.speaking.load(std::sync::atomic::Ordering::Relaxed),
        hyprland: config::is_hyprland(),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub fn get_status(app: AppHandle) -> Status {
    status_of(&app)
}

/// Persist settings and immediately re-apply whatever they affect.
#[tauri::command]
pub async fn save_config(app: AppHandle, config: Config) -> Result<String, String> {
    let previous = {
        let state = app.state::<AppState>();
        let previous = state.config();
        *state.cfg.lock().map_err(|_| "config lock poisoned")? = config.clone();
        previous
    };

    config.save().map_err(|e| format!("could not save settings: {e}"))?;

    // Editing bindings, or the service doing the transcribing, must not leave a
    // half-finished recording behind.
    if previous.hotkey != config.hotkey
        || previous.mode != config.mode
        || previous.provider != config.provider
    {
        app.state::<AppState>().stop_session();
    }

    let note = hotkeys::sync(&app, &config).unwrap_or_else(|e| e.to_string());

    if previous.launch_at_login != config.launch_at_login {
        if let Err(e) = set_launch_at_login(config.launch_at_login) {
            return Ok(format!("{note} — but autostart could not be changed: {e}"));
        }
    }

    if let Some(hud) = app.get_webview_window("hud") {
        if config.hud {
            let _ = hud.set_always_on_top(true);
        } else {
            let _ = hud.hide();
        }
    }

    let _ = app.emit("status", ());
    Ok(note)
}

#[tauri::command]
pub async fn start_dictation(app: AppHandle) -> Result<(), String> {
    state::start_dictation(app, Purpose::Dictate).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_dictation(app: AppHandle) {
    app.state::<AppState>().stop_session();
}

#[tauri::command]
pub async fn toggle_dictation(app: AppHandle) -> Result<(), String> {
    if app.state::<AppState>().is_recording() {
        app.state::<AppState>().stop_session();
        Ok(())
    } else {
        state::start_dictation(app, Purpose::Dictate).await.map_err(|e| e.to_string())
    }
}

/// Read text aloud with Deepgram Aura.
#[tauri::command]
pub fn speak(app: AppHandle, text: String) -> Result<(), String> {
    state::speak(&app, text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_speaking(app: AppHandle) {
    state::stop_speaking(&app);
}

#[tauri::command]
pub fn get_history(app: AppHandle) -> Vec<Entry> {
    app.state::<AppState>()
        .history
        .lock()
        .map(|h| h.iter().rev().cloned().collect())
        .unwrap_or_default()
}

/// Aggregates over the whole history, for the Insights page.
///
/// Computed here rather than in the frontend so the streak and rate arithmetic
/// is covered by `cargo test` alongside the rest of the Rust.
#[tauri::command]
pub fn get_insights(app: AppHandle) -> crate::stats::Insights {
    let history = app
        .state::<AppState>()
        .history
        .lock()
        .map(|h| h.clone())
        .unwrap_or_default();
    crate::stats::insights(&history, chrono::Local::now().date_naive())
}

#[tauri::command]
pub fn clear_history(app: AppHandle) {
    state::clear_history(&app);
}

#[tauri::command]
pub fn delete_history(app: AppHandle, id: String) {
    state::delete_history(&app, &id);
}

/// Paste a past transcript into whatever is focused right now.
#[tauri::command]
pub async fn reinject(app: AppHandle, id: String) -> Result<(), String> {
    let entry = app
        .state::<AppState>()
        .history
        .lock()
        .map_err(|_| "history lock poisoned")?
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or("that transcript is no longer in the history")?;

    let cfg = app.state::<AppState>().config();
    tokio::task::spawn_blocking(move || inject::deliver(&entry.text, &cfg))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_mics() -> Vec<String> {
    crate::audio::input_devices()
}

/// Step the dictation language, for the button on the Dictate screen.
#[tauri::command]
pub fn cycle_language(app: AppHandle) -> Result<String, String> {
    state::cycle_language(&app, false).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_language(app: AppHandle, code: String) -> Result<(), String> {
    state::set_language(&app, &code).map_err(|e| e.to_string())
}

/// Check the configured provider's key against that provider.
#[tauri::command]
pub fn verify_key(app: AppHandle) -> Result<String, String> {
    let cfg = app.state::<AppState>().config();
    let key = cfg.api_key().ok_or_else(|| {
        format!("No {} API key yet. Add it to .env or paste it above.", cfg.provider.label())
    })?;
    let check = match cfg.provider {
        Provider::Deepgram => deepgram::verify_key,
        Provider::AssemblyAi => assemblyai::verify_key,
        Provider::Gemini => gemini::verify_key,
    };
    check(&key).map_err(|e| e.to_string())
}

/// Write the push-to-talk bind into the Hyprland config right now.
#[tauri::command]
pub fn apply_hotkey(app: AppHandle) -> Result<String, String> {
    let cfg = app.state::<AppState>().config();
    hotkeys::sync(&app, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

/// Create or remove an XDG autostart entry. Re-applied at every launch when
/// enabled, so it follows the binary if it moves.
///
/// ponytail: Linux only. macOS needs a LaunchAgent and Windows a registry key;
/// add those when those builds actually ship.
pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = enabled;
        return Err("autostart is not implemented on this platform yet".into());
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
            Some(d) => std::path::PathBuf::from(d),
            None => std::path::PathBuf::from(home).join(".config"),
        }
        .join("autostart");
        let file = dir.join("orra.desktop");

        if !enabled {
            let _ = std::fs::remove_file(&file);
            return Ok(());
        }

        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Orra\n\
             Comment=Voice dictation for your whole desktop\n\
             Exec={}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&file, entry).map_err(|e| e.to_string())
    }
}
