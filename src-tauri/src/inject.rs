//! Put the finished transcript into whatever window has focus.

#[cfg(not(target_os = "windows"))]
use std::io::Write;
#[cfg(not(target_os = "windows"))]
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::config::{Config, Injection};

/// Terminals paste with Ctrl+Shift+V, everything else with Ctrl+V.
///
/// Only Linux can tell what has focus, so this is the only platform the list is
/// read on — plus the tests, which check the matching on every platform.
#[cfg(any(target_os = "linux", test))]
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

#[cfg(not(target_os = "windows"))]
fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(not(target_os = "windows"))]
fn run(cmd: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        // Captured rather than discarded: the reason these tools fail is
        // almost always a permission, and it is said on stderr. Swallowing it
        // turned "System Events is not allowed to send keystrokes" into the app
        // quietly typing nothing.
        .stderr(Stdio::piped())
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
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.trim();
        return Err(anyhow!(
            "{cmd} failed{}",
            if why.is_empty() { String::new() } else { format!(": {why}") }
        ));
    }
    Ok(out.stdout)
}

#[cfg(not(target_os = "windows"))]
fn which(cmd: &str) -> bool {
    // `Command::new` on a missing binary surfaces as NotFound, so probing the
    // usual directories avoids spawning a shell just to ask.
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
//
// All of this used to be a `powershell` or `clip` subprocess. Every one of them
// flashed a console window over whatever the user was typing into, and each had
// a bug of its own:
//
//   * `[System.Windows.Forms.SendKeys]::SendWait('^v')` needs that assembly
//     loaded first, and a bare `powershell -NoProfile -Command` has not loaded
//     it. The paste failed with "unable to find type" on a stderr we discard,
//     so nothing was ever inserted and nothing said why.
//   * `clip` reads its input in the OEM code page, so the UTF-8 transcript
//     reached the clipboard as mojibake — the app window looked perfect because
//     it had never gone through the clipboard at all.
//
// The Win32 calls underneath do the same job with no process, no window and no
// code page in between.
#[cfg(target_os = "windows")]
mod win {
    use std::time::Duration;

    use anyhow::{anyhow, Result};
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_RETURN, VK_V,
    };

    const CF_UNICODETEXT: u32 = 13;

    /// How many input events to hand `SendInput` at once.
    ///
    /// One call for a whole transcript would be simpler, but the call fails
    /// outright if the input queue fills up mid-way, and it is the queue the
    /// target application is draining. Chunking keeps each call small enough to
    /// land whole.
    const CHUNK: usize = 128;

    /// The clipboard is one global resource and any process may be mid-write on
    /// it, so a refusal is normal traffic rather than an error worth surfacing.
    fn open_clipboard() -> bool {
        for _ in 0..10 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    pub fn clipboard_get() -> Option<Vec<u8>> {
        if !open_clipboard() {
            return None;
        }
        let text = unsafe {
            let handle = GetClipboardData(CF_UNICODETEXT);
            if handle.is_null() {
                None
            } else {
                let ptr = GlobalLock(handle) as *const u16;
                if ptr.is_null() {
                    None
                } else {
                    // The text is NUL-terminated and `GlobalSize` is not
                    // reliable here, so scan for the end.
                    let mut len = 0;
                    while *ptr.add(len) != 0 {
                        len += 1;
                    }
                    let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                    GlobalUnlock(handle);
                    Some(s)
                }
            }
        };
        unsafe { CloseClipboard() };
        // UTF-8 in, UTF-8 out, so it survives the round trip through the
        // caller's `Vec<u8>` unchanged whatever the text contains.
        text.map(String::into_bytes)
    }

    pub fn clipboard_set(data: &[u8]) -> Result<()> {
        let text = String::from_utf8_lossy(data);
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);

        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            if handle.is_null() {
                return Err(anyhow!("could not allocate clipboard memory"));
            }
            let ptr = GlobalLock(handle) as *mut u16;
            if ptr.is_null() {
                GlobalFree(handle);
                return Err(anyhow!("could not lock clipboard memory"));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            GlobalUnlock(handle);

            if !open_clipboard() {
                GlobalFree(handle);
                return Err(anyhow!("another program is holding the clipboard"));
            }
            EmptyClipboard();
            // A successful `SetClipboardData` hands the block to the clipboard,
            // which frees it in turn — freeing it here as well would be a
            // double free, so only the failure path does.
            let stored = !SetClipboardData(CF_UNICODETEXT, handle).is_null();
            CloseClipboard();
            if !stored {
                GlobalFree(handle);
                return Err(anyhow!("the clipboard refused the text"));
            }
        }
        Ok(())
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// A character carried by its own UTF-16 code unit, which needs no key
    /// mapping — so unlike `SendKeys` this reaches non-ASCII text, and unlike a
    /// layout-dependent virtual key it does not care what the layout is.
    fn unicode(unit: u16, up: bool) -> INPUT {
        let mut flags = KEYEVENTF_UNICODE;
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send(events: &[INPUT]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let sent = unsafe {
            SendInput(
                events.len() as u32,
                events.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent as usize != events.len() {
            return Err(anyhow!("the system rejected the synthesized keystrokes"));
        }
        Ok(())
    }

    fn send_all(events: &[INPUT]) -> Result<()> {
        for chunk in events.chunks(CHUNK) {
            send(chunk)?;
        }
        Ok(())
    }

    pub fn paste() -> Result<()> {
        send(&[
            key(VK_CONTROL, false),
            key(VK_V, false),
            key(VK_V, true),
            key(VK_CONTROL, true),
        ])
    }

    pub fn enter() -> Result<()> {
        send(&[key(VK_RETURN, false), key(VK_RETURN, true)])
    }

    pub fn backspace(n: usize) -> Result<()> {
        let mut events = Vec::with_capacity(n * 2);
        for _ in 0..n {
            events.push(key(VK_BACK, false));
            events.push(key(VK_BACK, true));
        }
        send_all(&events)
    }

    pub fn type_text(text: &str) -> Result<()> {
        let mut events = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            events.push(unicode(unit, false));
            events.push(unicode(unit, true));
        }
        send_all(&events)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A surrogate pair has to come out as the two code units it went in as,
        /// which is what `encode_utf16` gives us and what `SendInput` expects.
        #[test]
        fn typing_keeps_surrogate_pairs_whole() {
            let input: Vec<u16> = "a😀".encode_utf16().collect();
            assert_eq!(input.len(), 3);
            let events: Vec<INPUT> = input.iter().flat_map(|u| [unicode(*u, false), unicode(*u, true)]).collect();
            assert_eq!(events.len(), 6);
            // Down then up for every unit, and the scan unit is the character.
            for pair in events.chunks(2) {
                let (down, up) = unsafe { (pair[0].Anonymous.ki, pair[1].Anonymous.ki) };
                assert_eq!(down.wScan, up.wScan);
                assert_eq!(down.dwFlags & KEYEVENTF_KEYUP, 0);
                assert_eq!(up.dwFlags & KEYEVENTF_KEYUP, KEYEVENTF_KEYUP);
                assert_eq!(down.dwFlags & KEYEVENTF_UNICODE, KEYEVENTF_UNICODE);
            }
        }

        /// Round-tripping through `Vec<u8>` must not touch the text: this is the
        /// step `clip` used to corrupt.
        #[test]
        fn clipboard_bytes_survive_a_utf8_round_trip() {
            for text in ["hello", "نویسه", "café — naïve", "😀"] {
                let bytes = text.to_string().into_bytes();
                let back = String::from_utf8_lossy(&bytes).into_owned();
                assert_eq!(back, text);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
//
// Keys are posted straight to the window server instead of being typed through
// `osascript` and System Events. The old path needed Automation *and*
// Accessibility, cost a process per keystroke, and — because the child's exit
// status was ignored — failed silently when a permission was missing, which
// looks exactly like the app typing nothing at all.
//
// Posting synthetic events needs one permission, Accessibility; without it the
// events are dropped on the floor, so every entry point here checks first and
// says what to do rather than pretending it worked.
#[cfg(target_os = "macos")]
mod mac {
    use anyhow::{anyhow, Result};
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // Virtual keycodes from <HIToolbox/Events.h>. They are physical positions,
    // not characters, so they do not change between keyboards or layouts.
    const KEY_V: u16 = 9;
    const KEY_RETURN: u16 = 36;
    const KEY_BACKSPACE: u16 = 51;

    /// How many UTF-16 units go into one typed event. The API takes a whole
    /// string, but a long one is silently truncated on the way to some apps, so
    /// the text is fed through in chunks that are known to survive intact.
    const TYPE_CHUNK: usize = 20;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        /// Whether this process may post synthetic events — that is, whether the
        /// user has turned it on under Privacy & Security → Accessibility.
        fn AXIsProcessTrusted() -> bool;
    }

    fn trusted() -> Result<()> {
        if unsafe { AXIsProcessTrusted() } {
            return Ok(());
        }
        // Sent to the right pane rather than only described: the permission has
        // to be given by hand, and finding it is the tedious part. Once per run,
        // so a second failed dictation does not reopen the window.
        static OPENED: std::sync::Once = std::sync::Once::new();
        OPENED.call_once(|| {
            let _ = std::process::Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                .spawn();
        });
        Err(anyhow!(
            "macOS has not allowed Orra to type. Turn Orra on in System Settings → \
             Privacy & Security → Accessibility, then dictate again."
        ))
    }

    fn source() -> Result<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| anyhow!("could not create the keyboard event source"))
    }

    /// Press and release `keycode`, `times` times, carrying `flags` both ways.
    fn press(keycode: u16, flags: CGEventFlags, times: usize) -> Result<()> {
        trusted()?;
        let source = source()?;
        for _ in 0..times.max(1) {
            for keydown in [true, false] {
                let event = CGEvent::new_keyboard_event(source.clone(), keycode, keydown)
                    .map_err(|_| anyhow!("could not build a key event"))?;
                event.set_flags(flags);
                event.post(CGEventTapLocation::HID);
            }
        }
        Ok(())
    }

    pub fn paste() -> Result<()> {
        press(KEY_V, CGEventFlags::CGEventFlagCommand, 1)
    }

    pub fn enter() -> Result<()> {
        press(KEY_RETURN, CGEventFlags::empty(), 1)
    }

    pub fn backspace(n: usize) -> Result<()> {
        press(KEY_BACKSPACE, CGEventFlags::empty(), n)
    }

    /// Type the text itself, leaving the clipboard alone.
    ///
    /// A key event can carry a Unicode string directly, so this does not need to
    /// go through the clipboard the way it used to — which is the whole point of
    /// **Type it out** for the apps that mangle a paste.
    pub fn type_text(text: &str) -> Result<()> {
        trusted()?;
        let source = source()?;
        let mut chunk = String::new();
        let mut units = 0;
        for ch in text.chars() {
            let width = ch.len_utf16();
            if units + width > TYPE_CHUNK && units > 0 {
                post_chunk(&source, &chunk)?;
                chunk.clear();
                units = 0;
            }
            chunk.push(ch);
            units += width;
        }
        if !chunk.is_empty() {
            post_chunk(&source, &chunk)?;
        }
        Ok(())
    }

    fn post_chunk(source: &CGEventSource, chunk: &str) -> Result<()> {
        // Keycode 0 is not a real key; the string set on the event is what gets
        // inserted, and the keydown is what makes the window server deliver it.
        let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| anyhow!("could not build a key event"))?;
        event.set_string(chunk);
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
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
        return win::clipboard_get();
    }
    #[allow(unreachable_code)]
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
        return win::clipboard_set(data);
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
#[cfg(target_os = "linux")]
fn focused_is_terminal() -> bool {
    let class = focused_class().to_lowercase();
    !class.is_empty() && TERMINALS.iter().any(|t| class.contains(t))
}

fn paste_keystroke() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let shift = focused_is_terminal();
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
        return mac::paste();
    }
    #[cfg(target_os = "windows")]
    {
        return win::paste();
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
    #[cfg(target_os = "windows")]
    {
        return win::type_text(text);
    }
    #[cfg(target_os = "macos")]
    {
        return mac::type_text(text);
    }
    #[allow(unreachable_code)]
    Err(anyhow!("unsupported platform"))
}

fn paste_text(text: &str, restore: bool) -> Result<()> {
    // Only read the old clipboard when it is going back: with restore off the
    // dictated text is meant to stay put, and reading it would be a clipboard
    // round trip nothing ever looks at.
    let previous = if restore { clipboard_get() } else { None };
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
        return mac::enter();
    }
    #[cfg(target_os = "windows")]
    {
        return win::enter();
    }
    #[allow(unreachable_code)]
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
        return mac::backspace(n);
    }
    #[cfg(target_os = "windows")]
    {
        return win::backspace(n);
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
