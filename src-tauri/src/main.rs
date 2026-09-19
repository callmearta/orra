// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assemblyai;
mod audio;
mod commands;
mod config;
mod deepgram;
mod gemini;
mod hotkeys;
mod hypr;
mod inject;
mod polish;
mod state;
mod stats;
mod stt;
mod translate;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager};

use state::{AppState, Purpose};

/// WebKitGTK's DMABUF renderer dies with "Error 71 (Protocol error) dispatching
/// to Wayland display" on NVIDIA — the whole app exits the moment the webview is
/// created. Turning it off is the documented workaround.
///
/// Only done where the driver is actually loaded: the fallback renderer is
/// slower, and there is no reason to pay for it on hardware that works. An
/// explicit `WEBKIT_DISABLE_DMABUF_RENDERER` in the environment always wins.
#[cfg(target_os = "linux")]
fn avoid_webkit_dmabuf_crash() {
    use std::path::Path;
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return;
    }
    let nvidia = Path::new("/sys/module/nvidia").exists()
        || Path::new("/proc/driver/nvidia/version").exists();
    if nvidia {
        // Safe here: nothing else has started, so no other thread can be reading
        // the environment while we write it.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
        println!("[orra] NVIDIA detected — using WebKit's fallback renderer");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    avoid_webkit_dmabuf_crash();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            commands::show_main(app.clone());
        }))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::save_config,
            commands::start_dictation,
            commands::stop_dictation,
            commands::toggle_dictation,
            commands::speak,
            commands::stop_speaking,
            commands::get_history,
            commands::get_insights,
            commands::clear_history,
            commands::delete_history,
            commands::reinject,
            commands::list_mics,
            commands::cycle_language,
            commands::set_language,
            commands::verify_key,
            commands::apply_hotkey,
            commands::show_main,
            commands::quit,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg = config::Config::load();
            let (port, token) = (cfg.port, cfg.token.clone());
            app.manage(AppState::new(cfg.clone()));

            // Register push-to-talk first. On Hyprland this writes the managed
            // blocks and reloads, and the HUD rule must be in place *before* the
            // HUD window is created — window rules only apply on creation, so a
            // later write would leave the overlay centred and unfocusable-proofed.
            match hotkeys::sync(&handle, &cfg) {
                Ok(note) => println!("[orra] {note}"),
                Err(e) => eprintln!("[orra] hotkey not registered: {e}"),
            }

            fit_main_to_screen(app);
            build_hud(&handle, cfg.hud)?;
            build_tray(&handle)?;
            watch_state_for_hud(&handle);

            // Keep the autostart entry pointing at wherever the binary lives now.
            if cfg.launch_at_login {
                let _ = commands::set_launch_at_login(true);
            }

            spawn_control_server(handle, port, token);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window keeps dictation running in the
            // background; quit from the tray or the settings screen.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Orra");
}

/// Keep the settings window inside the screen it opened on.
///
/// The configured size is chosen for a roomy desktop, but a 900px-tall floor is
/// taller than a 768px laptop panel, where it would push the window off the
/// bottom of the display. Both numbers here are logical pixels — the same unit
/// CSS uses — so a HiDPI screen is handled by its scale factor rather than
/// needing constants of its own. The config stays the single source of truth;
/// this only ever lowers what it asks for.
fn fit_main_to_screen(app: &tauri::App) {
    let Some(window) = app.get_webview_window("main") else { return };
    let Some(configured) = app.config().app.windows.iter().find(|w| w.label == "main") else {
        return;
    };
    // Before it is mapped the window may not know its monitor yet, so fall back
    // to the primary one rather than skipping the clamp entirely.
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let screen = monitor.size().to_logical::<f64>(monitor.scale_factor());

    let floor_w = configured.min_width.unwrap_or(0.0).min(screen.width);
    let floor_h = configured.min_height.unwrap_or(0.0).min(screen.height);
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(floor_w, floor_h)));

    // Nor should it open larger than the screen.
    let (want_w, want_h) = (
        configured.width.min(screen.width),
        configured.height.min(screen.height),
    );
    if want_w < configured.width || want_h < configured.height {
        let _ = window.set_size(tauri::LogicalSize::new(want_w, want_h));
    }
}

/// Small always-on-top overlay: a white pill with a level meter and a status dot.
///
/// Larger than the pill it draws. WebKitGTK clamps any window holding a webview
/// to roughly 200x290 — asking for less silently yields that — so the pill is
/// drawn centred inside this window and the rest is transparent. Clicks pass
/// through the transparent part, otherwise an invisible 200x290 rectangle would
/// swallow them over the bottom of the screen.
///
/// Nothing needs to know this size: the pill is centred inside whatever it gets,
/// and the window rule positions it using `window_w`/`window_h` as the compositor
/// sees them, so a different clamp elsewhere only changes the slack around it.
fn build_hud(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    let hud = tauri::WebviewWindowBuilder::new(app, "hud", tauri::WebviewUrl::App("hud.html".into()))
        .title("Orra HUD")
        .inner_size(200.0, 290.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(false)
        .visible(false)
        .build()?;

    if visible {
        // Shown on demand when a dictation starts; nothing to do until then.
        let _ = hud.hide();
    }
    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let dictate = MenuItem::with_id(app, "dictate", "Start / stop dictation", true, None::<&str>)?;
    let read = MenuItem::with_id(app, "read", "Read last transcript aloud", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Orra", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&dictate, &read, &settings, &quit])?;

    let mut builder = TrayIconBuilder::with_id("orra").menu(&menu).tooltip("Orra");
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "dictate" => {
                if app.state::<AppState>().is_recording() {
                    app.state::<AppState>().stop_session();
                } else {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = state::start_dictation(app.clone(), Purpose::Dictate).await {
                            let _ = app.emit(stt::EVT_ERROR, e.to_string());
                        }
                    });
                }
            }
            "read" => {
                let last = app
                    .state::<AppState>()
                    .history
                    .lock()
                    .ok()
                    .and_then(|h| h.last().map(|e| e.text.clone()));
                if let Some(text) = last {
                    if let Err(e) = state::speak(app, text) {
                        let _ = app.emit(stt::EVT_ERROR, e.to_string());
                    }
                }
            }
            "settings" => commands::show_main(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Show the HUD for exactly as long as the key is held.
///
/// Releasing it takes the overlay away at once — the rest of the dictation is
/// still in flight behind it (the audio is being flushed to Deepgram and the
/// text has yet to be typed), but that is no reason to keep a window on screen
/// over whatever the text is about to land in. The falling tone is what marks
/// the end of a dictation now.
fn watch_state_for_hud(app: &AppHandle) {
    let handle = app.clone();
    app.listen(stt::EVT_STATE, move |event| {
        let Ok(phase) = serde_json::from_str::<String>(event.payload()) else { return };
        if !handle.state::<AppState>().config().hud {
            return;
        }
        let Some(hud) = handle.get_webview_window("hud") else { return };

        if phase == "recording" {
            let _ = hud.show();
            // The window is mostly transparent slack around the pill, and without
            // this the invisible part would intercept clicks aimed at whatever is
            // behind it. It has to come after the first show: on Wayland tao
            // panics on an unrealised window (`window.window().unwrap()`), and the
            // overlay is created hidden.
            //
            // Where it appears is the compositor's business — the window rule
            // written by `hotkeys::sync` positions it as it opens. Nothing here
            // should move it afterwards: doing that means it is mapped centred and
            // then jumps, which is visible, and it needs coordinate arithmetic the
            // app has no reliable way to do across display scaling.
            let _ = hud.set_ignore_cursor_events(true);
        } else {
            // Both of the other phases: the key is already up.
            let _ = hud.hide();
        }
    });
}

// ---------------------------------------------------------------------------
// local control socket
// ---------------------------------------------------------------------------
//
// The Hyprland binds cannot call into a running process, so they run
// `orra-ctl`, which speaks this tiny line protocol over loopback.

fn spawn_control_server(app: AppHandle, port: u16, token: String) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "[orra] cannot listen on 127.0.0.1:{port} ({e}). \
                     The push-to-talk key will not work — is another copy running?"
                );
                return;
            }
        };
        for stream in listener.incoming().flatten() {
            let (app, token) = (app.clone(), token.clone());
            std::thread::spawn(move || handle_ctl(app, stream, &token));
        }
    });
}

fn handle_ctl(app: AppHandle, mut stream: TcpStream, token: &str) {
    // A client that dies between lines must not pin this thread forever.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let Ok(reader_stream) = stream.try_clone() else { return };
    let mut reader = BufReader::new(reader_stream);
    let mut header = String::new();
    if reader.read_line(&mut header).is_err() {
        return;
    }

    let mut parts = header.split_whitespace();
    let (sent_token, command) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if sent_token != token {
        let _ = writeln!(stream, "err: bad token");
        return;
    }

    let reply = match command {
        "start" => match tauri::async_runtime::block_on(state::start_dictation(app.clone(), Purpose::Dictate)) {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("err: {e}"),
        },
        "stop" => {
            app.state::<AppState>().stop_session();
            "ok".to_string()
        }
        "toggle" => {
            if app.state::<AppState>().is_recording() {
                app.state::<AppState>().stop_session();
                "ok".to_string()
            } else {
                match tauri::async_runtime::block_on(state::start_dictation(app.clone(), Purpose::Dictate)) {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("err: {e}"),
                }
            }
        }
        "translate" => {
            if app.state::<AppState>().is_recording() {
                app.state::<AppState>().stop_session();
                "ok".to_string()
            } else {
                match tauri::async_runtime::block_on(state::start_dictation(
                    app.clone(),
                    Purpose::Translate,
                )) {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("err: {e}"),
                }
            }
        }
        "lang-next" | "lang-prev" => {
            let backwards = command == "lang-prev";
            match state::cycle_language(&app, backwards) {
                Ok(code) => format!("language: {} ({code})", state::language_label(&code)),
                Err(e) => format!("err: {e}"),
            }
        }
        "lang" => {
            // The code follows the command word: `orra-ctl lang fa`.
            let mut code = String::new();
            let _ = reader.read_line(&mut code);
            let code = code.trim().to_string();
            if code.is_empty() {
                "err: lang needs a language code".to_string()
            } else {
                match state::set_language(&app, &code) {
                    Ok(()) => format!("language: {} ({code})", state::language_label(&code)),
                    Err(e) => format!("err: {e}"),
                }
            }
        }
        "speak" => {
            // Everything after the command word is the text to read.
            let mut text = String::new();
            let _ = reader.read_line(&mut text);
            match state::speak(&app, text.trim().to_string()) {
                Ok(()) => "ok".to_string(),
                Err(e) => format!("err: {e}"),
            }
        }
        "open" => {
            commands::show_main(app.clone());
            "ok".to_string()
        }
        "status" => format!("recording={}", app.state::<AppState>().is_recording()),
        "quit" => {
            app.exit(0);
            "ok".to_string()
        }
        other => format!("err: unknown command {other:?}"),
    };
    let _ = writeln!(stream, "{reply}");
}
