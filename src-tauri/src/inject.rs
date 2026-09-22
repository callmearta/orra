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

/// Run one of the typing tools, on the host when there is a sandbox in the way.
///
/// Inside a Flatpak the tools that do this work are not in the runtime — wtype,
/// ydotool, xdotool, wl-copy are the machine's, installed by the user or by the
/// deb. Running them through `flatpak-spawn --host` means the app types exactly
/// the way the unsandboxed build does, with the same tool and the same
/// behaviour, instead of a second implementation of the same idea that can be
/// subtly worse. It is one extra process per injection, not per keystroke.
#[cfg(not(target_os = "windows"))]
fn run(cmd: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut command = if crate::config::flatpak_id().is_some() {
        let mut spawned = Command::new("flatpak-spawn");
        // `setsid`, and it is not decoration. `wl-copy` does not hold the
        // selection itself — it leaves a process behind to do it, and that
        // process is what an application reads when it pastes. Anything
        // `flatpak-spawn` starts is killed when the command it ran returns, so
        // the process holding the selection died the moment the copy
        // "succeeded": the clipboard was correct for a moment and then reverted
        // to whatever was there before, which is why the text sometimes arrived
        // and mostly did not. A new session is out of that reach, so the
        // selection outlives the call that set it.
        spawned.arg("--host").arg("setsid").arg(cmd);
        spawned
    } else {
        Command::new(cmd)
    };
    let mut child = command
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
    #[cfg(target_os = "linux")]
    if crate::config::flatpak_id().is_some() {
        return host_has(cmd);
    }
    // `Command::new` on a missing binary surfaces as NotFound, so probing the
    // usual directories avoids spawning a shell just to ask.
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

/// Whether the *machine* has a command, asked once and remembered.
///
/// Inside a sandbox the app's own PATH says nothing about what the host has —
/// the tools are all out there and none of them are in here — and the answer is
/// wanted on the way to every keystroke. So it is asked once, in a single call
/// that checks all of them, and the answer is kept.
#[cfg(target_os = "linux")]
fn host_has(cmd: &str) -> bool {
    use std::sync::OnceLock;
    static HOST_TOOLS: OnceLock<Vec<String>> = OnceLock::new();

    let tools = HOST_TOOLS.get_or_init(|| {
        let probe = "for t in wtype ydotool xdotool wl-copy wl-paste xclip; do command -v \"$t\"; done";
        Command::new("flatpak-spawn")
            .args(["--host", "sh", "-c", probe])
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter_map(|line| line.rsplit('/').next().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    });
    tools.iter().any(|t| t == cmd)
}

// ---------------------------------------------------------------------------
// uinput
// ---------------------------------------------------------------------------
//
// The last resort on Linux, and the only thing that works inside a Flatpak.
//
// Flatpak stamps its Wayland connection with a security context, and
// compositors hide the privileged protocols from clients carrying one.
// `zwp_virtual_keyboard_manager_v1` — wtype's entire mechanism — is among them,
// which is why bundling wtype in a sandbox achieves nothing. Sway and the rest
// of wlroots filter the same way, and Mutter and KWin never implemented the
// protocol at all, so there was never a compositor where bundling it helped.
//
// uinput is a different road: the app asks the *kernel* for a keyboard, and the
// compositor picks it up through libinput exactly as it would a real one. That
// works from inside a sandbox, identically on every compositor, and needs no
// host binary — at the cost of `--device=all` in the Flatpak manifest.
//
// It types through the *active* keyboard layout, so it can only reach
// characters that layout has keys for. `type_text` handles that by handing
// anything non-ASCII to the clipboard instead.
#[cfg(target_os = "linux")]
mod uinput {
    use std::fs::{File, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use anyhow::{anyhow, Result};

    /// How long a key is held down, and the pause after releasing it.
    ///
    /// A press and release written in the same instant can be coalesced away
    /// before the compositor ever sees a keystroke. `wtype -d 2` paces itself
    /// for the same reason; this is slower because it is a system call per
    /// event rather than one batch.
    const HOLD: Duration = Duration::from_millis(8);
    const GAP: Duration = Duration::from_millis(6);

    /// How long to let the compositor notice a freshly created keyboard.
    ///
    /// Device enumeration is asynchronous — udev, then libinput, then the
    /// compositor's own hotplug — and events sent before it finishes go
    /// nowhere. `warm_up` is what keeps this off the critical path; it is the
    /// backstop for the paths that reach a keyboard cold.
    const SETTLE: Duration = Duration::from_millis(400);

    // ioctl requests from <linux/uinput.h>. libc carries neither the _IO/_IOW
    // macros nor these constants, so they are written out from the macro
    // itself: _IOC(dir, type, nr, size) packs the four fields as
    // (dir << 30) | (size << 16) | (type << 8) | nr, where a _IOW has dir 1,
    // 'U' is uinput's type byte, and the sizes are the widths the header
    // declares. `ioctl_numbers_match_the_kernel_header` pins them to the values
    // that header produces.
    const fn io(nr: libc::c_ulong) -> libc::c_ulong {
        ((b'U' as libc::c_ulong) << 8) | nr
    }
    const fn iow(nr: libc::c_ulong, size: libc::c_ulong) -> libc::c_ulong {
        (1 << 30) | (size << 16) | ((b'U' as libc::c_ulong) << 8) | nr
    }

    // There is no UI_DEV_DESTROY here: the keyboard lives as long as the
    // process, and the kernel tears it down when the fd closes on exit.
    const UI_DEV_CREATE: libc::c_ulong = io(1);
    const UI_DEV_SETUP: libc::c_ulong = iow(3, size_of::<Setup>() as libc::c_ulong);
    const UI_SET_EVBIT: libc::c_ulong = iow(100, 4);
    const UI_SET_KEYBIT: libc::c_ulong = iow(101, 4);

    // <linux/input-event-codes.h>. Only what this module can emit.
    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const EV_REP: u16 = 0x14;
    const SYN_REPORT: u16 = 0;
    const BUS_USB: u16 = 0x03;

    const KEY_BACKSPACE: u16 = 14;
    const KEY_TAB: u16 = 15;
    const KEY_ENTER: u16 = 28;
    const KEY_LEFTCTRL: u16 = 29;
    const KEY_LEFTSHIFT: u16 = 42;

    /// Every key that produces printable ASCII on a US layout, as
    /// `(keycode, unshifted, shifted)`.
    ///
    /// The layout the compositor is actually running decides what these
    /// keycodes mean; this table is what makes the *intent* of a character
    /// expressible at all, and is why non-ASCII cannot go this way.
    const ASCII_KEYS: &[(u16, char, char)] = &[
        (2, '1', '!'),
        (3, '2', '@'),
        (4, '3', '#'),
        (5, '4', '$'),
        (6, '5', '%'),
        (7, '6', '^'),
        (8, '7', '&'),
        (9, '8', '*'),
        (10, '9', '('),
        (11, '0', ')'),
        (12, '-', '_'),
        (13, '=', '+'),
        (16, 'q', 'Q'),
        (17, 'w', 'W'),
        (18, 'e', 'E'),
        (19, 'r', 'R'),
        (20, 't', 'T'),
        (21, 'y', 'Y'),
        (22, 'u', 'U'),
        (23, 'i', 'I'),
        (24, 'o', 'O'),
        (25, 'p', 'P'),
        (26, '[', '{'),
        (27, ']', '}'),
        (30, 'a', 'A'),
        (31, 's', 'S'),
        (32, 'd', 'D'),
        (33, 'f', 'F'),
        (34, 'g', 'G'),
        (35, 'h', 'H'),
        (36, 'j', 'J'),
        (37, 'k', 'K'),
        (38, 'l', 'L'),
        (39, ';', ':'),
        (40, '\'', '"'),
        (41, '`', '~'),
        (43, '\\', '|'),
        (44, 'z', 'Z'),
        (45, 'x', 'X'),
        (46, 'c', 'C'),
        (47, 'v', 'V'),
        (48, 'b', 'B'),
        (49, 'n', 'N'),
        (50, 'm', 'M'),
        (51, ',', '<'),
        (52, '.', '>'),
        (53, '/', '?'),
        (57, ' ', ' '),
    ];

    /// `struct uinput_setup`, which is `input_id` plus a fixed-width name.
    #[repr(C)]
    struct Setup {
        id: libc::input_id,
        name: [u8; 80],
        ff_effects_max: u32,
    }

    struct Keyboard(File);

    /// The virtual keyboard, created once and then held open.
    ///
    /// It has to outlive each injection: tearing the device down after every
    /// transcript would turn each one into a hotplug, and anything sent while
    /// the compositor was still enumerating it would be dropped on the floor.
    static KEYBOARD: OnceLock<Mutex<Option<Keyboard>>> = OnceLock::new();

    impl Keyboard {
        fn create() -> Result<Self> {
            let file = OpenOptions::new().write(true).open("/dev/uinput")?;
            let kb = Self(file);
            kb.ioctl(UI_SET_EVBIT, EV_KEY)?;
            kb.ioctl(UI_SET_EVBIT, EV_SYN)?;
            kb.ioctl(UI_SET_EVBIT, EV_REP)?;
            // Declared up front because the kernel rejects keycodes that were
            // never announced — a missing bit would only surface mid-transcript.
            for code in ASCII_KEYS
                .iter()
                .map(|(code, _, _)| *code)
                .chain([KEY_BACKSPACE, KEY_TAB, KEY_ENTER, KEY_LEFTCTRL, KEY_LEFTSHIFT])
            {
                kb.ioctl(UI_SET_KEYBIT, code as libc::c_ulong)?;
            }

            let mut setup = Setup {
                id: libc::input_id {
                    bustype: BUS_USB,
                    vendor: 0x1d6b,
                    product: 0x0001,
                    version: 1,
                },
                name: [0; 80],
                ff_effects_max: 0,
            };
            let name = b"orra virtual keyboard";
            setup.name[..name.len()].copy_from_slice(name);
            kb.ioctl_ptr(UI_DEV_SETUP, &setup)?;
            kb.ioctl(UI_DEV_CREATE, 0u8)?;
            std::thread::sleep(SETTLE);
            Ok(kb)
        }

        /// The arg is spelled as whatever the request wants — a bit number, a
        /// keycode, or nothing at all — and widened to the register width the
        /// variadic call passes it in.
        fn ioctl(&self, request: libc::c_ulong, arg: impl Into<libc::c_ulong>) -> Result<()> {
            if unsafe { libc::ioctl(self.0.as_raw_fd(), request as _, arg.into()) } < 0 {
                return Err(anyhow!("uinput ioctl {request:#x}: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }

        fn ioctl_ptr<T>(&self, request: libc::c_ulong, arg: &T) -> Result<()> {
            if unsafe { libc::ioctl(self.0.as_raw_fd(), request as _, arg as *const T) } < 0 {
                return Err(anyhow!("uinput ioctl {request:#x}: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }

        fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<()> {
            let event = libc::input_event {
                time: libc::timeval { tv_sec: 0, tv_usec: 0 },
                type_,
                code,
                value,
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(&event as *const libc::input_event as *const u8, size_of::<libc::input_event>())
            };
            use std::io::Write;
            self.0.write_all(bytes)?;
            Ok(())
        }

        /// Terminate a packet. Events written without one are not acted on.
        fn sync(&mut self) -> Result<()> {
            self.emit(EV_SYN, SYN_REPORT, 0)
        }

        /// Press keys in order, hold, then release them in reverse.
        fn chord(&mut self, held: &[u16], code: u16) -> Result<()> {
            for key in held {
                self.emit(EV_KEY, *key, 1)?;
            }
            self.emit(EV_KEY, code, 1)?;
            self.sync()?;
            std::thread::sleep(HOLD);
            self.emit(EV_KEY, code, 0)?;
            for key in held.iter().rev() {
                self.emit(EV_KEY, *key, 0)?;
            }
            self.sync()?;
            std::thread::sleep(GAP);
            Ok(())
        }

        fn tap(&mut self, code: u16) -> Result<()> {
            self.chord(&[], code)
        }
    }

    /// Run `f` against the keyboard, creating it on first use.
    fn with_keyboard<T>(f: impl FnOnce(&mut Keyboard) -> Result<T>) -> Result<T> {
        let slot = KEYBOARD.get_or_init(|| Mutex::new(Keyboard::create().ok()));
        // A panic while another thread held the lock must not cost the user
        // every later transcript.
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let keyboard = guard
            .as_mut()
            .ok_or_else(|| anyhow!("no keyboard on /dev/uinput — the sandbox needs --device=all"))?;
        f(keyboard)
    }

    /// Build the keyboard now rather than on the first transcript, if it is the
    /// only thing on this machine that can type.
    ///
    /// Enumeration takes a moment — udev, then libinput, then the compositor —
    /// and a transcript delivered during it would be typed into nothing. That
    /// is a bad thing to discover on the first thing you say, so it is paid for
    /// at startup instead. Machines that have wtype or xdotool never pay it,
    /// and never get an extra keyboard in their device list.
    pub fn warm_up_if_needed() {
        if is_needed() {
            let _ = with_keyboard(|_| Ok(()));
        }
    }

    fn is_needed() -> bool {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        if wayland && (super::which("wtype") || super::which("ydotool")) {
            return false;
        }
        !super::which("xdotool")
    }

    fn ascii_key(c: char) -> Option<(u16, bool)> {
        ASCII_KEYS.iter().find_map(|(code, plain, shifted)| {
            if c == *plain {
                Some((*code, false))
            } else if c == *shifted && shifted != plain {
                Some((*code, true))
            } else {
                None
            }
        })
    }

    /// Type text that the active layout can produce. Callers must have checked
    /// `text.is_ascii()` — anything else has no keycode here.
    pub fn type_ascii(text: &str) -> Result<()> {
        with_keyboard(|kb| {
            for c in text.chars() {
                let (code, shift) = ascii_key(c)
                    .ok_or_else(|| anyhow!("no key on this layout types {c:?}"))?;
                let held: &[u16] = if shift { &[KEY_LEFTSHIFT] } else { &[] };
                kb.chord(held, code)?;
            }
            Ok(())
        })
    }

    /// Ctrl+V, or Ctrl+Shift+V for a terminal.
    ///
    /// The chord is the same physical key on every layout, which is what makes
    /// clipboard paste the reliable mode in a sandbox.
    pub fn paste(shift: bool) -> Result<()> {
        let code = ascii_key('v').expect("v is in the table").0;
        let held: &[u16] = if shift {
            &[KEY_LEFTCTRL, KEY_LEFTSHIFT]
        } else {
            &[KEY_LEFTCTRL]
        };
        with_keyboard(|kb| kb.chord(held, code))
    }

    pub fn enter() -> Result<()> {
        with_keyboard(|kb| kb.tap(KEY_ENTER))
    }

    pub fn backspace(n: usize) -> Result<()> {
        with_keyboard(|kb| {
            for _ in 0..n {
                kb.tap(KEY_BACKSPACE)?;
            }
            Ok(())
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The ioctl numbers are derived by hand from the macro, so pin them to
        /// what <linux/uinput.h> actually produces for this struct layout.
        #[test]
        fn ioctl_numbers_match_the_kernel_header() {
            assert_eq!(UI_DEV_CREATE, 0x5501);
            assert_eq!(UI_SET_EVBIT, 0x40045564);
            assert_eq!(UI_SET_KEYBIT, 0x40045565);
            assert_eq!(size_of::<Setup>(), 92);
            assert_eq!(UI_DEV_SETUP, 0x405c5503);
        }

        /// Every printable ASCII character has to resolve to a key that
        /// actually produces it.
        #[test]
        fn every_printable_ascii_character_has_a_key() {
            for byte in 0x20u8..=0x7e {
                let c = byte as char;
                let (code, shift) = ascii_key(c).unwrap_or_else(|| panic!("{c:?} has no key"));
                assert!(
                    ASCII_KEYS.iter().any(|(k, plain, shifted)| {
                        *k == code && if shift { *shifted == c } else { *plain == c }
                    }),
                    "{c:?} resolved to a key that does not produce it"
                );
            }
        }

        /// …and no two characters may share a keycode and shift state, or one
        /// of them would silently type the other.
        #[test]
        fn no_two_characters_share_a_key() {
            let mut seen: Vec<(u16, bool)> = Vec::new();
            for (code, plain, shifted) in ASCII_KEYS {
                let chars = if plain == shifted {
                    vec![*plain]
                } else {
                    vec![*plain, *shifted]
                };
                for c in chars {
                    let key = (*code, c != *plain);
                    assert!(!seen.contains(&key), "{c:?} collides on {key:?}");
                    seen.push(key);
                }
            }
            assert_eq!(seen.len(), 0x7e - 0x20 + 1);
        }
    }
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
            // Asked over Hyprland's own socket rather than through this
            // module's `run`, so it keeps working where hyprctl cannot be
            // shipped — see `crate::hypr::ask`.
            if let Ok(out) = crate::hypr::ask("j/activewindow") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
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

/// One way of asking the session to type something.
#[cfg(target_os = "linux")]
struct Attempt<'a> {
    cmd: &'a str,
    args: Vec<&'a str>,
    stdin: Option<&'a [u8]>,
}

/// Run the first attempt that works, in the order given.
///
/// A tool that is not installed is skipped. A tool that runs and *fails* is
/// remembered and the next one is tried, which is the part that makes this work
/// on more than one kind of desktop: `wtype` is installed on plenty of GNOME
/// and KDE machines and fails on every one of them, because those compositors
/// never implemented the protocol it speaks. Treating "installed" as "will
/// work" left those users with an app that transcribed and then typed nothing.
///
/// `None` means every tool was missing, and the caller should fall through to
/// the kernel keyboard. `Some(Err(..))` means tools were there and every one of
/// them refused — worth reporting rather than papering over, because that is
/// the shape of a real permission problem rather than a missing package.
#[cfg(target_os = "linux")]
fn first_that_works(attempts: &[Attempt]) -> Option<Result<()>> {
    let mut refused = None;
    for attempt in attempts {
        if !which(attempt.cmd) {
            continue;
        }
        match run(attempt.cmd, &attempt.args, attempt.stdin) {
            Ok(_) => return Some(Ok(())),
            Err(e) => refused = Some(e),
        }
    }
    refused.map(Err)
}

fn paste_keystroke() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let shift = focused_is_terminal();
        let mut wtype = vec!["-M", "ctrl"];
        if shift {
            wtype.extend(["-M", "shift"]);
        }
        wtype.extend(["-k", "v"]);

        let attempts = if is_wayland() {
            let ydotool = if shift { "42:1 29:1 47:1 47:0 29:0 42:0" } else { "29:1 47:1 47:0 29:0" };
            vec![
                Attempt { cmd: "wtype", args: wtype, stdin: None },
                Attempt { cmd: "ydotool", args: vec!["key", ydotool], stdin: None },
            ]
        } else {
            let combo = if shift { "ctrl+shift+v" } else { "ctrl+v" };
            vec![Attempt { cmd: "xdotool", args: vec!["key", "--clearmodifiers", combo], stdin: None }]
        };
        if let Some(outcome) = first_that_works(&attempts) {
            return outcome;
        }
        // Nothing on PATH can type, which is exactly what a sandbox looks like:
        // no host binaries at all. The kernel keyboard is the one thing that is
        // always there, so it is the floor rather than an error.
        return uinput::paste(shift);
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
        // The machine's wtype types anything, so unlike a synthetic keyboard
        // this needs no clipboard detour for non-ASCII.
        if crate::config::flatpak_id().is_some() {
            return host_type(text);
        }
        // `-` reads the text from stdin, so a long transcript cannot overflow argv.
        let wtype = Attempt { cmd: "wtype", args: vec!["-d", "2", "-"], stdin: Some(text.as_bytes()) };
        let xdotool = Attempt {
            cmd: "xdotool",
            args: vec!["type", "--clearmodifiers", "--", text],
            stdin: None,
        };
        // xdotool is a candidate on Wayland too: it cannot reach a native
        // Wayland window, but it can reach an X11 one under Xwayland.
        let attempts: &[Attempt] = if is_wayland() { &[wtype, xdotool] } else { &[xdotool] };
        if let Some(outcome) = first_that_works(attempts) {
            return outcome;
        }
        // The kernel keyboard types through whatever layout is active, so it
        // can only reach the characters that layout has keys for — wtype could
        // do better because it uploads a keymap of its own. Anything past
        // ASCII therefore goes the long way round: the text rides the
        // clipboard and only the Ctrl+V is synthesized. The old clipboard is
        // put back, because this is "type" mode and it promises to leave no
        // trace on it.
        if text.is_ascii() {
            return uinput::type_ascii(text);
        }
        return paste_text(text, true);
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

/// How long to wait either side of the paste.
///
/// The second number is the one that shows. Putting the old clipboard back
/// before the target application has read the selection replaces the
/// transcript with whatever was there before, so the paste lands as stale text
/// or as nothing at all — and which applications survive that is a matter of
/// how quickly each reads the selection, which is exactly what "works in some
/// apps but not others" looks like.
///
/// It is worse inside a sandbox, where each of these steps is another process
/// to spawn through the sandbox machinery, so the margins are widened there.
fn clipboard_settle() -> (Duration, Duration) {
    if crate::config::flatpak_id().is_some() {
        (Duration::from_millis(180), Duration::from_millis(900))
    } else {
        (Duration::from_millis(60), Duration::from_millis(220))
    }
}

fn paste_text(text: &str, restore: bool) -> Result<()> {
    // In a sandbox the whole paste goes to the machine in one piece, because
    // every step of it is a spawn and the windows between them are where the
    // wrong clipboard gets pasted. See `HOST_INJECT_SCRIPT`.
    #[cfg(target_os = "linux")]
    if crate::config::flatpak_id().is_some() {
        return host_paste(text, restore);
    }
    // Only read the old clipboard when it is going back: with restore off the
    // dictated text is meant to stay put, and reading it would be a clipboard
    // round trip nothing ever looks at.
    let (before_paste, before_restore) = clipboard_settle();
    let previous = if restore { clipboard_get() } else { None };
    clipboard_set(text.as_bytes())?;
    // Give the clipboard owner a moment to claim the selection before pasting.
    std::thread::sleep(before_paste);
    paste_keystroke()?;
    // Let the target application read the clipboard before we put it back.
    std::thread::sleep(before_restore);
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
        if crate::config::flatpak_id().is_some() {
            let helper = crate::config::config_dir().join(HOST_INJECT);
            return run(&helper.display().to_string(), &["enter"], None).map(|_| ());
        }
        let wtype = Attempt { cmd: "wtype", args: vec!["-k", "Return"], stdin: None };
        let xdotool = Attempt { cmd: "xdotool", args: vec!["key", "--clearmodifiers", "Return"], stdin: None };
        let attempts: &[Attempt] = if is_wayland() { &[wtype, xdotool] } else { &[xdotool] };
        if let Some(outcome) = first_that_works(attempts) {
            return outcome;
        }
        return uinput::enter();
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
        if crate::config::flatpak_id().is_some() {
            let helper = crate::config::config_dir().join(HOST_INJECT);
            let count = n.to_string();
            return run(&helper.display().to_string(), &["backspace", &count], None).map(|_| ());
        }
        let mut backspaces: Vec<&str> = Vec::new();
        for _ in 0..n {
            backspaces.push("-k");
            backspaces.push("BackSpace");
        }
        let spec = format!("BackSpace Repeat:{n}");
        let wtype = Attempt { cmd: "wtype", args: backspaces, stdin: None };
        let xdotool = Attempt {
            cmd: "xdotool",
            args: vec!["key", "--clearmodifiers", &spec],
            stdin: None,
        };
        let attempts: &[Attempt] = if is_wayland() { &[wtype, xdotool] } else { &[xdotool] };
        if let Some(outcome) = first_that_works(attempts) {
            return outcome;
        }
        return uinput::backspace(n);
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

/// Get the injection path ready before the first transcript needs it.
pub fn warm_up() {
    #[cfg(target_os = "linux")]
    {
        uinput::warm_up_if_needed();
        if crate::config::flatpak_id().is_some() {
            let _ = write_host_helper();
        }
    }
}

// ---------------------------------------------------------------------------
// the host-side injector
// ---------------------------------------------------------------------------
//
// Inside a Flatpak every step of a paste is a separate `flatpak-spawn`, and
// that is what made it unreliable rather than merely slow.
//
// `wl-copy` does not hold the selection itself: it leaves a process behind to
// do it, and an application pastes whatever that process is offering at the
// moment it reads. Between the copy and the keystroke there is a window where
// the *previous* clipboard is still the one on offer — paste inside it and the
// wrong text is typed, which is exactly what "it pasted an old clipboard entry"
// is. The same window opens at the other end: put the old clipboard back too
// soon and a slow application reads that instead.
//
// Both are timing, and timing across a D-Bus round trip is not something to
// guess at with a sleep. So the whole sequence happens in one process on the
// machine, and it waits for the selection to actually be the transcript before
// sending the keystroke.

/// The injector written for the host to run.
#[cfg(target_os = "linux")]
const HOST_INJECT: &str = "orra-inject";

/// Where the transcript is put for the helper to read.
///
/// A file rather than stdin: it is one less thing to get wrong across the
/// spawn, and it is already in a directory the host can see.
#[cfg(target_os = "linux")]
fn pending_path() -> std::path::PathBuf {
    crate::config::config_dir().join("orra-pending")
}

#[cfg(target_os = "linux")]
fn write_host_helper() -> Result<()> {
    let path = crate::config::config_dir().join(HOST_INJECT);
    std::fs::write(&path, HOST_INJECT_SCRIPT)
        .map_err(|e| anyhow!("writing {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

/// Written to the app's config directory, where the host can run it.
#[cfg(target_os = "linux")]
const HOST_INJECT_SCRIPT: &str = r#"#!/bin/bash
# Written by Orra. Runs on the machine, not in the sandbox.
#
#   paste <file> <shift> <restore>   put the file on the clipboard, paste it, maybe put the old one back
#   type <file>                      type the file out
#   enter                            press Return
#   backspace <n>                    press Backspace n times
#
# The waiting here is not padding. wl-copy hands the selection to a process it
# leaves behind, and until that lands the clipboard still holds what it held
# before — pasting in that gap types the previous contents. So this waits for
# the selection to actually be the text, rather than sleeping and hoping.
set -u

wait_for_selection() {
  want="$1"
  for _ in $(seq 1 60); do
    [ "$(wl-paste --no-newline 2>/dev/null || true)" = "$want" ] && return 0
    sleep 0.05
  done
  return 1
}

case "${1:-}" in
  paste)
    file="$2"; shift_key="$3"; restore="$4"
    want="$(cat "$file")"
    previous=""
    [ "$restore" = 1 ] && previous="$(wl-paste --no-newline 2>/dev/null || true)"

    printf '%s' "$want" | setsid wl-copy
    wait_for_selection "$want" || exit 1

    if [ "$shift_key" = 1 ]; then
      wtype -M ctrl -M shift -k v
    else
      wtype -M ctrl -k v
    fi

    # Long enough for an application to have read the selection, short enough
    # not to be felt. Restoring before that is the other half of the same bug.
    sleep 0.5
    if [ "$restore" = 1 ] && [ -n "$previous" ]; then
      printf '%s' "$previous" | setsid wl-copy
    fi
    rm -f "$file"
    ;;
  type)
    wtype -d 2 - < "$2"
    rm -f "$2"
    ;;
  enter)
    wtype -k Return
    ;;
  backspace)
    i=0
    while [ "$i" -lt "${2:-1}" ]; do wtype -k BackSpace; i=$((i + 1)); done
    ;;
esac
"#;

/// Hand the paste to the machine, whole.
#[cfg(target_os = "linux")]
fn host_paste(text: &str, restore: bool) -> Result<()> {
    let file = pending_path();
    std::fs::write(&file, text)?;
    let helper = crate::config::config_dir().join(HOST_INJECT);
    let shift = if focused_is_terminal() { "1" } else { "0" };
    let restore = if restore { "1" } else { "0" };
    run(
        &helper.display().to_string(),
        &["paste", &file.display().to_string(), shift, restore],
        None,
    )
    .map(|_| ())
}

/// Type the text out on the machine.
#[cfg(target_os = "linux")]
fn host_type(text: &str) -> Result<()> {
    let file = pending_path();
    std::fs::write(&file, text)?;
    let helper = crate::config::config_dir().join(HOST_INJECT);
    run(&helper.display().to_string(), &["type", &file.display().to_string()], None).map(|_| ())
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

    /// The whole point of the ladder: a tool that is installed but useless must
    /// not stop the ones below it from being tried. `true` and `false` stand in
    /// for a working tool and a refusing one, so this exercises the real spawn
    /// path rather than a mock of it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_missing_tool_is_skipped_and_a_working_one_after_it_is_used() {
        let missing = || Attempt { cmd: "orra-no-such-tool", args: vec![], stdin: None };
        let works = || Attempt { cmd: "true", args: vec![], stdin: None };

        assert!(matches!(first_that_works(&[missing(), works()]), Some(Ok(()))));
        assert!(matches!(first_that_works(&[works(), missing()]), Some(Ok(()))));
    }

    /// A tool that refuses is reported only once nothing else can take over —
    /// that is a real error worth showing, unlike "not installed", which is
    /// just the sandbox and is handled by falling through to uinput.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_refusing_tool_is_reported_only_when_nothing_else_is_left() {
        let refuses = || Attempt { cmd: "false", args: vec![], stdin: None };
        let works = || Attempt { cmd: "true", args: vec![], stdin: None };

        assert!(matches!(first_that_works(&[refuses()]), Some(Err(_))));
        assert!(matches!(first_that_works(&[refuses(), works()]), Some(Ok(()))));
    }

    /// Nothing installed at all is `None`, which is what tells the callers to
    /// reach for the kernel keyboard instead of giving up.
    #[cfg(target_os = "linux")]
    #[test]
    fn nothing_installed_leaves_it_to_the_caller() {
        let missing = || Attempt { cmd: "orra-no-such-tool", args: vec![], stdin: None };
        assert!(first_that_works(&[missing()]).is_none());
        assert!(first_that_works(&[]).is_none());
    }

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
