//! Put the finished transcript into whatever window has focus.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::config::{Config, Injection};

/// Terminals paste with Ctrl+Shift+V, everything else with Ctrl+V.
const TERMINALS: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "ghostty",
    "xterm",
    "konsole",
    "gnome-terminal",
    "org.gnome.terminal",
    "st",
    "rio",
    "contour",
    "tmux",
    "blackbox",
    "warp",
];

fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn run(cmd: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("{cmd}: {e}"))?;
    if let Some(data) = stdin {
        child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("{cmd}: no stdin"))?
            .write_all(data)?;
    }
    let out = child.wait_with_output()?;
    Ok(out.stdout)
}

fn which(cmd: &str) -> bool {
    // `Command::new` on a missing binary surfaces as NotFound, so probing the
    // usual directories avoids spawning a shell just to ask.
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// clipboard
// ---------------------------------------------------------------------------

fn clipboard_get() -> Option<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        if is_wayland() && which("wl-paste") {
            return run("wl-paste", &["--no-newline"], None).ok();
        }
        if which("xclip") {
            return run("xclip", &["-o", "-selection", "clipboard"], None).ok();
        }
    }
    #[cfg(target_os = "macos")]
    {
        return run("pbpaste", &[], None).ok();
    }
    #[cfg(target_os = "windows")]
    {
        return run("powershell", &["-NoProfile", "-Command", "Get-Clipboard -Raw"], None).ok();
    }
    None
}

fn clipboard_set(data: &[u8]) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if is_wayland() && which("wl-copy") {
            return run("wl-copy", &[], Some(data)).map(|_| ());
        }
        if which("xclip") {
            return run("xclip", &["-selection", "clipboard"], Some(data)).map(|_| ());
        }
        return Err(anyhow!("no clipboard tool found (install wl-clipboard)"));
    }
    #[cfg(target_os = "macos")]
    {
        return run("pbcopy", &[], Some(data)).map(|_| ());
    }
    #[cfg(target_os = "windows")]
    {
        return run("clip", &[], Some(data)).map(|_| ());
    }
    #[allow(unreachable_code)]
    Err(anyhow!("unsupported platform"))
}

// ---------------------------------------------------------------------------
// keystrokes
// ---------------------------------------------------------------------------

/// Window class of the focused window, for history labels and terminal detection.
pub fn focused_class() -> String {
    #[cfg(target_os = "linux")]
    {
        if crate::config::is_hyprland() {
            if let Ok(out) = run("hyprctl", &["activewindow", "-j"], None) {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out) {
                    if let Some(class) = v.get("class").and_then(|c| c.as_str()) {
                        if !class.is_empty() {
                            return class.to_string();
                        }
                    }
                }
            }
        }
        // X11 fallback, and the best we can do when nothing is focused.
        if which("xdotool") {
            if let Ok(out) = run("xdotool", &["getactivewindow", "getwindowclassname"], None) {
                return String::from_utf8_lossy(&out).trim().to_string();
            }
        }
        String::new()
    }
    #[cfg(not(target_os = "linux"))]
    {
        String::new()
    }
}

/// Terminals paste with Ctrl+Shift+V rather than Ctrl+V.
fn focused_is_terminal() -> bool {
    let class = focused_class().to_lowercase();
    !class.is_empty() && TERMINALS.iter().any(|t| class.contains(t))
}

fn paste_keystroke() -> Result<()> {
    let shift = focused_is_terminal();

    #[cfg(target_os = "linux")]
    {
        if is_wayland() {
            if which("wtype") {
                let mut args = vec!["-M", "ctrl"];
                if shift {
                    args.extend(["-M", "shift"]);
                }
                args.extend(["-k", "v"]);
                return run("wtype", &args, None).map(|_| ());
            }
            if which("ydotool") {
                let keys = if shift { "42:1 29:1 47:1 47:0 29:0 42:0" } else { "29:1 47:1 47:0 29:0" };
                return run("ydotool", &["key", keys], None).map(|_| ());
            }
            return Err(anyhow!("no key injection tool found (install wtype)"));
        }
        if which("xdotool") {
            let combo = if shift { "ctrl+shift+v" } else { "ctrl+v" };
            return run("xdotool", &["key", "--clearmodifiers", combo], None).map(|_| ());
        }
        return Err(anyhow!("no key injection tool found (install xdotool)"));
    }
    #[cfg(target_os = "macos")]
    {
        let script = r#"tell application "System Events" to keystroke "v" using command down"#;
        return run("osascript", &["-e", script], None).map(|_| ());
    }
    #[cfg(target_os = "windows")]
    {
        let script = "[System.Windows.Forms.SendKeys]::SendWait('^v')";
        return run("powershell", &["-NoProfile", "-Command", script], None).map(|_| ());
    }
    #[allow(unreachable_code)]
    Err(anyhow!("unsupported platform"))
}

/// Type the characters directly, leaving the clipboard alone.
fn type_text(text: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if is_wayland() && which("wtype") {
            // `-` reads the text from stdin, so a long transcript cannot overflow argv.
            return run("wtype", &["-d", "2", "-"], Some(text.as_bytes())).map(|_| ());
        }
        if which("xdotool") {
            return run("xdotool", &["type", "--clearmodifiers", "--", text], None).map(|_| ());
        }
        return Err(anyhow!("no text injection tool found (install wtype)"));
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        // Typing arbitrary text through AppleScript/PowerShell is not reliable,
        // so those platforms always go through the clipboard.
        return paste_text(text, true);
    }
    #[allow(unreachable_code)]
    Err(anyhow!("unsupported platform"))
}

fn paste_text(text: &str, restore: bool) -> Result<()> {
    let previous = clipboard_get();
    clipboard_set(text.as_bytes())?;
    // Give the clipboard owner a moment to claim the selection before pasting.
    std::thread::sleep(Duration::from_millis(60));
    paste_keystroke()?;
    // Let the target application read the clipboard before we put it back.
    std::thread::sleep(Duration::from_millis(220));
    if restore {
        if let Some(prev) = previous {
            let _ = clipboard_set(&prev);
        }
    }
    Ok(())
}

pub fn press_enter() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if is_wayland() && which("wtype") {
            return run("wtype", &["-k", "Return"], None).map(|_| ());
        }
        if which("xdotool") {
            return run("xdotool", &["key", "--clearmodifiers", "Return"], None).map(|_| ());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let s = r#"tell application "System Events" to key code 36"#;
        return run("osascript", &["-e", s], None).map(|_| ());
    }
    #[cfg(target_os = "windows")]
    {
        let s = "[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')";
        return run("powershell", &["-NoProfile", "-Command", s], None).map(|_| ());
    }
    Err(anyhow!("no way to send Return on this platform"))
}

/// Simulate pressing Backspace `n` times, for "scratch that".
pub fn press_backspace(n: usize) -> Result<()> {
    if n == 0 {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        if is_wayland() && which("wtype") {
            let mut args: Vec<String> = Vec::new();
            for _ in 0..n {
                args.push("-k".into());
                args.push("BackSpace".into());
            }
            let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            return run("wtype", &refs, None).map(|_| ());
        }
        if which("xdotool") {
            let spec = format!("BackSpace Repeat:{}", n);
            return run("xdotool", &["key", "--clearmodifiers", &spec], None).map(|_| ());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let s = format!(
            r#"tell application "System Events" to repeat {n} times
                   key code 51
               end repeat"#
        );
        return run("osascript", &["-e", &s], None).map(|_| ());
    }
    #[cfg(target_os = "windows")]
    {
        let s = format!("[System.Windows.Forms.SendKeys]::SendWait('{}')", "{BS}".repeat(n));
        return run("powershell", &["-NoProfile", "-Command", &s], None).map(|_| ());
    }
    #[allow(unreachable_code)]
    Err(anyhow!("no way to send Backspace on this platform"))
}

/// Deliver the transcript. Blocking — call from a worker thread.
pub fn deliver(text: &str, cfg: &Config) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    match cfg.injection {
        Injection::Type => type_text(text),
        // Only put the old clipboard back if the setting asks for it — many
        // people would rather keep the dictated text on hand.
        Injection::ClipboardPaste => paste_text(text, cfg.restore_clipboard),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_classes_match_loosely() {
        for class in ["kitty", "Alacritty", "com.mitchellh.ghostty", "org.wezfurlong.wezterm"] {
            let lc = class.to_lowercase();
            assert!(
                TERMINALS.iter().any(|t| lc.contains(t)),
                "{class} should be treated as a terminal"
            );
        }
        assert!(!TERMINALS.iter().any(|t| "firefox".contains(t)));
    }

    #[test]
    fn empty_text_is_never_delivered() {
        assert!(deliver("", &Config::default()).is_ok());
    }
}
