//! Global hotkey registration for platforms where the OS allows it.
//!
//! On Hyprland this is a no-op: Wayland exposes no global-shortcut protocol, so
//! the bind lives in the compositor config instead (see [`crate::hypr`]).

use anyhow::{anyhow, Result};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcut, GlobalShortcutExt, Shortcut, ShortcutState};

use crate::config::{self, Config, Mode};
use crate::state::{AppState, Purpose};

/// What a hotkey event should do, once the recording mode is taken into account.
enum Act {
    Start,
    Stop,
    Nothing,
}

/// Translate our canonical `SUPER + ALT + D` form into the plugin's syntax
/// (`Super+Alt+KeyD`).
pub fn to_plugin_shortcut(hotkey: &str) -> Result<String> {
    let mut parts = Vec::new();
    for raw in hotkey.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let mapped = match token.to_ascii_uppercase().as_str() {
            "SUPER" | "META" | "WIN" | "CMD" | "COMMAND" => "Super".to_string(),
            "CTRL" | "CONTROL" => "Control".to_string(),
            "ALT" | "OPTION" => "Alt".to_string(),
            "SHIFT" => "Shift".to_string(),
            other if other.len() == 1 && other.chars().all(|c| c.is_ascii_alphabetic()) => {
                format!("Key{other}")
            }
            other if other.len() == 1 && other.chars().all(|c| c.is_ascii_digit()) => {
                format!("Digit{other}")
            }
            // Named keys need the plugin's own spelling, not Hyprland's.
            "SPACE" => "Space".to_string(),
            "RETURN" | "ENTER" => "Enter".to_string(),
            "TAB" => "Tab".to_string(),
            "ESCAPE" | "ESC" => "Escape".to_string(),
            "BACKSPACE" => "Backspace".to_string(),
            "UP" => "ArrowUp".to_string(),
            "DOWN" => "ArrowDown".to_string(),
            "LEFT" => "ArrowLeft".to_string(),
            "RIGHT" => "ArrowRight".to_string(),
            other if other.starts_with('F') && other[1..].parse::<u8>().is_ok() => {
                other.to_string()
            }
            other => other.to_string(),
        };
        parts.push(mapped);
    }
    if parts.is_empty() {
        return Err(anyhow!("hotkey is empty"));
    }
    Ok(parts.join("+"))
}

/// (Re)register the push-to-talk shortcuts. Returns a human-readable status.
pub fn sync(app: &AppHandle, cfg: &Config) -> Result<String> {
    if config::is_hyprland() {
        return crate::hypr::apply(cfg);
    }

    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    let mut registered = vec![bind(gs, &cfg.hotkey, cfg.mode, Purpose::Dictate)?];

    // The translating key is optional, and pointless when it is the same key
    // as the one that already dictates.
    let translate = cfg.translate_hotkey.trim();
    if !translate.is_empty() && translate != cfg.hotkey.trim() {
        registered.push(bind(gs, translate, cfg.mode, Purpose::Translate)?);
    }

    Ok(format!("Registered {} with the system", registered.join(" and ")))
}

/// Register one key, which starts a session of the given kind.
fn bind(
    gs: &GlobalShortcut<tauri::Wry>,
    hotkey: &str,
    mode: Mode,
    purpose: Purpose,
) -> Result<String> {
    let spec = to_plugin_shortcut(hotkey)?;
    let shortcut: Shortcut = spec
        .parse()
        .map_err(|e| anyhow!("{spec:?} is not a usable hotkey: {e}"))?;

    gs.on_shortcut(shortcut, move |app, _shortcut, event| {
        let recording = app.state::<AppState>().is_recording();
        let act = match (mode, event.state()) {
            (Mode::Hold, ShortcutState::Pressed) => Act::Start,
            (Mode::Hold, ShortcutState::Released) => Act::Stop,
            // Toggle reacts to the press only — the release must be inert,
            // otherwise a tap would start and immediately stop.
            (Mode::Toggle, ShortcutState::Pressed) if recording => Act::Stop,
            (Mode::Toggle, ShortcutState::Pressed) => Act::Start,
            (Mode::Toggle, ShortcutState::Released) => Act::Nothing,
        };

        match act {
            Act::Start => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::state::start_dictation(app.clone(), purpose).await {
                        use tauri::Emitter;
                        let _ = app.emit(crate::stt::EVT_ERROR, e.to_string());
                    }
                });
            }
            Act::Stop => app.state::<AppState>().stop_session(),
            Act::Nothing => {}
        }
    })
    .map_err(|e| anyhow!("could not register {spec}: {e}"))?;

    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_letters_and_digits_map_correctly() {
        assert_eq!(to_plugin_shortcut("SUPER + ALT + D").unwrap(), "Super+Alt+KeyD");
        assert_eq!(to_plugin_shortcut("CTRL+SHIFT+1").unwrap(), "Control+Shift+Digit1");
        assert_eq!(to_plugin_shortcut("alt + F5").unwrap(), "Alt+F5");
        assert!(to_plugin_shortcut("   ").is_err());
    }

    #[test]
    fn named_keys_use_the_plugin_spelling() {
        // The UI and the Hyprland block both feed this the same string.
        assert_eq!(to_plugin_shortcut("SUPER   +   SPACE").unwrap(), "Super+Space");
        assert_eq!(to_plugin_shortcut("CTRL+RETURN").unwrap(), "Control+Enter");
    }
}
