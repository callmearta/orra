// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assemblyai;
mod audio;
mod commands;
mod config;
mod deepgram;
mod engine;
mod gemini;
mod hotkeys;
mod hypr;
mod inject;
mod local;
mod polish;
mod problem;
mod state;
mod stats;
mod stt;
mod translate;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Listener, Manager};

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
            commands::verify_translate,
            commands::list_models,
            commands::local_availability,
            commands::download_local_model,
            commands::use_local_model,
            commands::stop_local_engine,
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

            // Bring up the local model this app is responsible for, if any. On
            // its own thread: starting it means loading the weights, which is
            // long enough that the window should not wait for it.
            engine::start_configured(handle.clone());

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

/// Let the HUD float over another app's fullscreen window.
///
/// A window that is merely always-on-top does not: macOS gives a fullscreen app
/// its own Space, and a window belongs to one Space unless it says otherwise.
/// The two behaviours that change that — joining every Space and being auxiliary
/// to a fullscreen one — have no setter in Tauri, so the NSWindow is asked
/// directly. The level is raised to the screen-saver one, which is above the
/// level a fullscreen app is drawn at.
#[cfg(target_os = "macos")]
fn make_hud_overlay(window: &tauri::WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let Ok(ptr) = window.ns_window() else { return };
    let ns_window: *mut AnyObject = ptr.cast();

    // From NSWindow.h: canJoinAllSpaces (1<<0), stationary (1<<4), and
    // fullScreenAuxiliary (1<<8).
    const BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 8);
    // NSScreenSaverWindowLevel.
    const SCREEN_SAVER_LEVEL: isize = 1000;

    unsafe {
        let _: () = msg_send![ns_window, setCollectionBehavior: BEHAVIOR];
        let _: () = msg_send![ns_window, setLevel: SCREEN_SAVER_LEVEL];
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
        // `focused(false)` only skips the focus it would take as it is created.
        // Showing it again for each dictation would still activate it, and an
        // overlay that takes focus takes the paste target with it — the same
        // thing the `no_focus` rule exists to prevent on Hyprland. Windows is
        // the only platform that honours this; elsewhere it is a no-op and the
        // compositor rule or window manager does the job.
        .focusable(false)
        .visible(false)
        .build()?;

    // macOS keeps a plain always-on-top window off another app's fullscreen
    // Space, so the window is told to join them and sit above them.
    #[cfg(target_os = "macos")]
    make_hud_overlay(&hud);

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
                            problem::report(&app, "Could not start dictating", e);
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
                        problem::report(app, "Could not read that aloud", e);
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

/// Put the overlay at the bottom centre of the screen being worked on.
///
/// Hyprland is told where to put it by a window rule; every other platform
/// leaves the toolkit's default placement, which drops the overlay wherever the
/// window manager happens to cascade it — nowhere near the bottom, and on a
/// second monitor not even on the right screen.
///
/// The cursor decides which screen. It is where the user's attention already
/// is, it is the screen the dictation is going into, and unlike the app's own
/// window it is meaningful even when the settings window is closed and only the
/// tray is left.
///
/// Every coordinate here is a physical pixel: `Monitor::size`,
/// `Monitor::position`, `outer_size` and `set_position` all speak that, so no
/// scale factor is needed to mix them. The margin and the pill height are the
/// exception — they are logical, because the pill is drawn by the webview in
/// CSS pixels — so they are scaled to match. This is the same arithmetic as the
/// Hyprland rule, which gets away with the unscaled numbers because Hyprland's
/// own coordinate space is logical too.
fn place_hud(app: &AppHandle, hud: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| hud.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, hud.outer_size()) else {
        return;
    };

    let (x, y) = hud_origin(
        (monitor.position().x, monitor.position().y),
        (monitor.size().width, monitor.size().height),
        (size.width, size.height),
        monitor.scale_factor(),
    );
    let _ = hud.set_position(tauri::PhysicalPosition::new(x, y));
}

/// The overlay's top-left corner, in physical pixels.
///
/// The window is far taller than the pill drawn inside it, and the pill sits
/// centred in whatever it got, so centring the *window* on where the pill's
/// centre belongs is what puts the pill on the margin.
fn hud_origin(monitor: (i32, i32), screen: (u32, u32), win: (u32, u32), scale: f64) -> (i32, i32) {
    let lift =
        ((hypr::HUD_BOTTOM_MARGIN as f64 + hypr::HUD_PILL_H as f64 / 2.0) * scale).round() as i32;
    (
        monitor.0 + (screen.0 as i32 - win.0 as i32) / 2,
        monitor.1 + screen.1 as i32 - lift - win.1 as i32 / 2,
    )
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
            // On Hyprland where it appears is the compositor's business — the
            // window rule written by `hotkeys::sync` positions it as it opens,
            // and moving it afterwards would map it centred and then jump. No
            // other platform has a rule to write, so there the app places it.
            if !config::is_hyprland() {
                place_hud(&handle, &hud);
            }
            // The window is mostly transparent slack around the pill, and without
            // this the invisible part would intercept clicks aimed at whatever is
            // behind it. It has to come after the first show: on Wayland tao
            // panics on an unrealised window (`window.window().unwrap()`), and the
            // overlay is created hidden.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the arithmetic: wherever the screen is and however it is
    /// scaled, the pill's bottom edge ends up `HUD_BOTTOM_MARGIN` above the
    /// screen's bottom edge. The pill is centred in the window and the window
    /// height cancels out of that sum, which is what makes the oversized
    /// webview window harmless.
    #[test]
    fn the_pill_lands_on_the_bottom_margin() {
        let win = (200, 290);
        for (screen, scale) in [
            ((1920u32, 1080u32), 1.0),
            ((3840, 2160), 2.0),
            ((2560, 1440), 1.5),
        ] {
            let (_, y) = hud_origin((0, 0), screen, win, scale);
            // Pill centre is the window centre; its bottom is half a pill below.
            let pill_bottom = y + win.1 as i32 / 2 + (hypr::HUD_PILL_H as f64 * scale / 2.0) as i32;
            let want = screen.1 as i32 - (hypr::HUD_BOTTOM_MARGIN as f64 * scale) as i32;
            assert!(
                (pill_bottom - want).abs() <= 1,
                "screen {screen:?} at {scale}x put the pill at {pill_bottom}, wanted {want}"
            );
        }
    }

    /// Centred horizontally, and offset by the monitor's own origin — a second
    /// screen to the left of the primary has a negative x, and the overlay has
    /// to follow it there rather than sit on the primary.
    #[test]
    fn the_overlay_follows_the_monitor_it_is_on() {
        let win = (200, 290);
        assert_eq!(hud_origin((0, 0), (1920, 1080), win, 1.0).0, 860);
        assert_eq!(hud_origin((-1920, 0), (1920, 1080), win, 1.0).0, -1060);
        assert_eq!(hud_origin((1920, 0), (1920, 1080), win, 1.0).0, 2780);
    }
}
