//! The command surface the settings UI talks to.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, Config, Provider};
use crate::deepgram;
use crate::problem::Problem;
use crate::{assemblyai, gemini};
use crate::hotkeys;
use crate::inject;
use crate::state::{self, AppState, Entry, Purpose};

#[derive(Serialize)]
pub struct Status {
    config: Config,
    /// What each provider is and what it can be asked for, so the settings
    /// screen draws the right fields without a second copy of those rules.
    providers: Vec<crate::config::ProviderInfo>,
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
        // The local provider usually has no key to have, and reporting that as
        // missing would read as "not set up yet" on a server that is running
        // perfectly well.
        has_key: cfg.provider.is_self_hosted() || cfg.api_key().is_some(),
        providers: Provider::all().into_iter().map(Into::into).collect(),
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

    // The engine this app runs belongs to the provider that uses it: switching
    // away leaves a process holding a gigabyte of memory for nothing, and
    // switching back should not mean finding the model again. Started on its
    // own thread either way — loading the weights is not something a settings
    // save waits for.
    if previous.provider != config.provider {
        match config.provider {
            Provider::Orra => crate::engine::start_configured(app.clone()),
            _ if previous.provider == Provider::Orra => crate::engine::stop(&app),
            _ => {}
        }
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
///
/// Off the invoke thread: a command that is not `async` runs inline in the
/// handler, and this one makes a network call. The call is bounded by
/// [`crate::problem::VERIFY_TIMEOUT`], but a bounded twenty seconds spent
/// blocking is still the whole window frozen for twenty seconds.
#[tauri::command]
pub async fn verify_key(app: AppHandle) -> Result<String, Problem> {
    let cfg = app.state::<AppState>().config();

    // A server the user runs has no key of its own to check and usually wants
    // none: what is being checked is the endpoint, so it is asked for its model
    // list — the same call the Settings button beside the model field makes.
    if cfg.provider.is_self_hosted() {
        let summary = format!("Checking the {} server", cfg.provider.label());
        return crate::local::check(&cfg)
            .await
            .map_err(|e| Problem::new(summary, e, &cfg));
    }

    let key = cfg.api_key().ok_or_else(|| {
        Problem::new(
            "No key to check",
            format!("No {} API key yet. Add it to .env or paste it in Settings.", cfg.provider.label()),
            &cfg,
        )
    })?;

    let check = match cfg.provider {
        Provider::Deepgram => deepgram::verify_key,
        Provider::AssemblyAi => assemblyai::verify_key,
        Provider::Gemini => gemini::verify_key,
        _ => unreachable!("self-hosted providers are answered above, before a key is asked for"),
    };
    // Cloned because the closure below takes `cfg`, and the error paths still
    // need it to name the failure.
    let (summary, cert) = (format!("Checking the {} key", cfg.provider.label()), cfg.clone());

    tokio::task::spawn_blocking(move || check(&key))
        .await
        .map_err(|e| Problem::new("The key check failed", e, &cert))?
        .map_err(|e| Problem::new(summary, e, &cert))
}

/// Check the translation service by asking it to translate something.
///
/// A real call rather than a listing: it is the only check that covers the
/// endpoint, the key and the model name together, which are the three things
/// that can be wrong — and it is the same code path a dictation takes, so a
/// pass here means translating will work.
///
/// Off the invoke thread for the same reason as [`verify_key`], and more so:
/// the endpoint may be a model on this machine, where a cold load is slow, and
/// one that accepts the connection and then says nothing waits out the full
/// timeout.
#[tauri::command]
pub async fn verify_translate(app: AppHandle) -> Result<String, Problem> {
    let cfg = app.state::<AppState>().config();
    let cert = cfg.clone();
    tokio::task::spawn_blocking(move || crate::translate::check(&cfg))
        .await
        .map_err(|e| Problem::new("The translation check failed", e, &cert))?
        .map_err(|e| Problem::new("Checking the translation service", e, &cert))
}

/// The models a server has, for the fields that fill themselves from it.
///
/// One command for both cards: the local transcription server and the custom
/// translation endpoint are the same kind of thing — an OpenAI-compatible
/// service — and both want the list of what is installed rather than a name
/// typed from memory. The URL and key come from the fields themselves rather
/// than from the saved settings, so the button works on what is on screen
/// before the debounced save has gone through.
///
/// Off the invoke thread: the endpoint may be a model on this machine that has
/// not been started yet, and this waits out the check timeout when it is not.
#[tauri::command]
pub async fn list_models(
    app: AppHandle,
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, Problem> {
    let cert = app.state::<AppState>().config();
    let redaction = cert.clone();
    tokio::task::spawn_blocking(move || crate::local::list_models(&base_url, &api_key))
        .await
        .map_err(|e| Problem::new("Listing the models failed", e, &redaction))?
        .map_err(|e| Problem::new("Listing the models on that server", e, &cert))
}

/// What the Settings card needs to offer running a model here.
#[tauri::command]
pub fn local_availability(app: AppHandle) -> crate::engine::Availability {
    crate::engine::availability(&app)
}

/// Fetch a model — and the engine, once — and leave them on disk.
///
/// Downloading and starting are separate on purpose: the first is a wait on the
/// user's connection with nothing to show for it but progress, and the second
/// is a choice they can make later, or undo with Stop without throwing the
/// weights away.
///
/// Off the invoke thread, because it is a gigabyte over that connection.
#[tauri::command]
pub async fn download_local_model(app: AppHandle, name: String) -> Result<String, Problem> {
    let cert = app.state::<AppState>().config();
    let handle = app.clone();
    let wanted = name.clone();
    tokio::task::spawn_blocking(move || crate::engine::download(&handle, &wanted))
        .await
        .map_err(|e| Problem::new("Downloading the local model failed", e, &cert))?
        .map_err(|e| Problem::new("Downloading the local model", e, &cert))?;

    let label = crate::engine::model(&name).map(|m| m.label).unwrap_or(&name);
    Ok(format!("{label} is downloaded. Choose it and press Use this model to start it."))
}

/// Start the engine on a model that is already downloaded, and point the
/// settings at it — the second half of the one-click path.
#[tauri::command]
pub async fn use_local_model(app: AppHandle, name: String) -> Result<String, Problem> {
    let cert = app.state::<AppState>().config();
    let handle = app.clone();
    let wanted = name.clone();
    // Starting it also writes the address it landed on into the settings: the
    // port is chosen fresh each time, so the one saved from last time is stale.
    let url = tokio::task::spawn_blocking(move || {
        let model = crate::engine::model(&wanted)
            .ok_or_else(|| anyhow::anyhow!("there is no local model called {wanted}"))?;
        crate::engine::start_and_record(&handle, model)
    })
    .await
    .map_err(|e| Problem::new("Starting the local model failed", e, &cert))?
    .map_err(|e| Problem::new("Starting the local model", e, &cert))?;

    let label = crate::engine::model(&name).map(|m| m.label).unwrap_or(&name);
    Ok(format!("{label} is running at {url}. Hold your key and speak."))
}

/// Stop the engine this app started. The model stays on disk.
#[tauri::command]
pub fn stop_local_engine(app: AppHandle) {
    crate::engine::stop(&app);
    let _ = app.emit("status", ());
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
    // The engine is this app's child process, and nothing else outlives it.
    crate::engine::stop(&app);
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
