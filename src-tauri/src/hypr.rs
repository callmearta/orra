//! Hyprland integration.
//!
//! Wayland has no global-shortcut protocol, so the push-to-talk bind has to be
//! registered by the compositor. This writes a managed block into the user's Lua
//! config and reloads Hyprland — the block is delimited so re-applying it never
//! duplicates or tramples hand-written binds.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::config::{self, Config, Mode};

const BEGIN: &str = "-- >>> orra (managed block - edits are overwritten) >>>";
const END: &str = "-- <<< orra <<<";

fn keybinds_path() -> PathBuf {
    config::hypr_lua_dir().join("keybinds.lua")
}

fn settings_path() -> PathBuf {
    config::hypr_lua_dir().join("settings.lua")
}

/// Absolute path to the control binary, so the bind does not depend on PATH.
pub fn ctl_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("orra-ctl")))
        .unwrap_or_else(|| PathBuf::from("orra-ctl"))
}

/// The command a bind runs, ready for the arguments to be appended.
///
/// Inside a Flatpak this cannot be a path. The bind is executed by Hyprland on
/// the *host*, where `/app/bin/orra-ctl` does not exist, so it goes back
/// through `flatpak run` — which re-enters the sandbox and finds the binary
/// beside the app, reading the sandbox's own config for the port and token.
/// A `flatpak run` costs about 60ms, which is paid once on the press and once
/// on the release: not enough to clip the first word, and far less than the
/// alternative of holding a second copy of the binary on the host, which would
/// have to be built against the host's libc to be runnable there.
pub fn ctl_invocation() -> String {
    match config::flatpak_id() {
        Some(id) => flatpak_invocation(&id),
        None => ctl_path().display().to_string(),
    }
}

/// Split from the environment lookup so the one string the bind depends on can
/// be checked without a sandbox to run in — this is what a compositor will
/// execute on every keypress, and it cannot be tested by hand from here.
fn flatpak_invocation(app_id: &str) -> String {
    format!("flatpak run --command=orra-ctl {app_id}")
}

/// Quote a value for a Lua string literal.
///
/// The hotkeys come from the settings screen as free text and are written
/// straight into the user's config, so a stray quote or backslash would not
/// just produce a broken bind — it would end the string and put the rest of the
/// line into the file as code. Escaping keeps whatever was typed inside the
/// literal, where a bad key is then a bind Hyprland rejects rather than Lua it
/// tries to run.
fn lua_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn keybind_block(cfg: &Config) -> String {
    let ctl = ctl_invocation();
    // A full `exec_cmd` argument: the quoted path plus the command word.
    fn exec(ctl: &str, args: &str) -> String {
        lua_string(&format!("{ctl} {args}"))
    }

    let key = lua_string(cfg.hotkey.trim());
    let dictate = match cfg.mode {
        Mode::Hold => format!(
            r#"hl.bind({key}, hl.dsp.exec_cmd({}), {{ description = "orra: dictate (hold)" }})
hl.bind({key}, hl.dsp.exec_cmd({}), {{ release = true, description = "orra: dictate (release)" }})"#,
            exec(&ctl, "start"),
            exec(&ctl, "stop"),
        ),
        Mode::Toggle => format!(
            r#"hl.bind({key}, hl.dsp.exec_cmd({}), {{ description = "orra: dictate (toggle)" }})"#,
            exec(&ctl, "toggle"),
        ),
    };

    let mut out = dictate;

    // The translating key: the same recording, translated before it is typed.
    let translate = cfg.translate_hotkey.trim();
    if !translate.is_empty() && translate != cfg.hotkey.trim() {
        let translate_key = lua_string(translate);
        out.push('\n');
        out.push_str(&match cfg.mode {
            Mode::Hold => format!(
                r#"hl.bind({translate_key}, hl.dsp.exec_cmd({}), {{ description = "orra: dictate and translate (hold)" }})
hl.bind({translate_key}, hl.dsp.exec_cmd({}), {{ release = true, description = "orra: translate (release)" }})"#,
                exec(&ctl, "translate"),
                exec(&ctl, "stop"),
            ),
            Mode::Toggle => format!(
                r#"hl.bind({translate_key}, hl.dsp.exec_cmd({}), {{ description = "orra: dictate and translate (toggle)" }})"#,
                exec(&ctl, "translate"),
            ),
        });
    }

    // Only bind a second key when there is something to switch between, and
    // only for a provider that takes a language: Gemini detects one itself and
    // would do nothing with this key.
    let lang = cfg.language_hotkey.trim();
    if cfg.provider.has_language() && !lang.is_empty() && cfg.language_cycle.len() > 1 {
        out.push('\n');
        out.push_str(&format!(
            r#"hl.bind({}, hl.dsp.exec_cmd({}), {{ description = "orra: next dictation language" }})"#,
            lua_string(lang),
            exec(&ctl, "lang-next"),
        ));
    }
    out
}

/// How far the pill floats above the bottom edge of the monitor, in pixels.
///
/// Logical pixels, like everything else in CSS and in Hyprland's coordinate
/// space. `place_hud` in main.rs scales them for the platforms where the app
/// has to position the overlay itself.
pub const HUD_BOTTOM_MARGIN: i64 = 120;

/// The pill's own height, as drawn by hud.html. The window around it is much
/// bigger (the toolkit will not make a webview window smaller), so the window
/// has to be lifted by the difference to put the pill where this margin says.
pub const HUD_PILL_H: i64 = 24;

/// Floating, pinned, never-focused overlay, positioned by the compositor.
///
/// `no_focus` matters for correctness, not just looks: if the HUD could take
/// focus it would steal the paste target out from under a dictation.
///
/// `move` is an expression the compositor evaluates as the window opens, so the
/// overlay is *born* in place rather than appearing centred and then jumping.
///
/// Every term is one of Hyprland's own variables, evaluated in Hyprland's own
/// coordinate space, which is what makes this portable: the app never converts
/// between screen and window coordinates itself. `hyprctl monitors` reports
/// *physical* pixels while window geometry is in *logical* pixels, so arithmetic
/// mixing the two is right on an unscaled display and wrong on a scaled one —
/// the kind of bug that only shows up on someone else's machine.
///
/// The variables are monitor-local, so this centres the overlay on whichever
/// monitor it lands on, at whatever resolution and aspect ratio that monitor has.
/// `lift` is how far the *window* has to sit above the monitor's bottom edge for
/// the pill drawn inside it to clear `HUD_BOTTOM_MARGIN`: the margin plus half the
/// pill, plus half the window (the `window_h/2` term).
fn hud_rule_block() -> String {
    let lift = HUD_BOTTOM_MARGIN + HUD_PILL_H / 2;
    format!(
        r#"hl.window_rule({{
  name = "orra-hud",
  match = {{ title = "^(Orra HUD)$" }},
  move = {{ "(monitor_w/2-window_w/2)", "(monitor_h-{lift}-window_h/2)" }},
  float = true,
  pin = true,
  no_focus = true,
  no_anim = true,
  no_blur = true,
  no_shadow = true,
  no_dim = true,
  border_size = 0,
  rounding = 0,
}})"#
    )
}

/// Register (or refresh) the bind and HUD rule, then reload Hyprland.
pub fn apply(cfg: &Config) -> Result<String> {
    if !config::is_hyprland() {
        return Ok("Not running under Hyprland — using the global shortcut plugin instead.".into());
    }

    // Before anything is written, not after: a compositor that cannot be
    // reached must not leave the user's config half-applied.
    reachable()?;

    write_block(&keybinds_path(), &keybind_block(cfg))?;
    if cfg.hud {
        write_block(&settings_path(), &hud_rule_block())?;
    } else {
        remove_block_file(&settings_path())?;
    }

    reload()?;

    // Surface parse errors instead of letting the user discover them later.
    let errs = ask("configerrors")?;
    let errs = errs.trim();
    if !errs.is_empty() && errs != "no errors" {
        return Ok(format!("Applied, but Hyprland reported config errors: {errs}"));
    }
    Ok(format!("Bound to {} in keybinds.lua", cfg.hotkey.trim()))
}

fn reload() -> Result<()> {
    ask("reload").map(|_| ())
}

/// Where Hyprland's command socket lives for this instance.
///
/// `XDG_RUNTIME_DIR` is the only place it can be, and the instance signature is
/// what keeps a second compositor from being talked to by mistake.
fn socket_path() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is not set"))?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .ok_or_else(|| anyhow!("HYPRLAND_INSTANCE_SIGNATURE is not set"))?;
    Ok(socket_in(Path::new(&runtime), &signature))
}

/// The layout itself, split out from the environment so it can be checked
/// without a compositor to read the variables from.
fn socket_in(runtime: &Path, signature: &std::ffi::OsStr) -> PathBuf {
    runtime.join("hypr").join(signature).join(".socket.sock")
}

/// Check that Hyprland can be reached at all, before its config is touched.
///
/// `hyprctl` used to fail *after* the managed block had already been written,
/// which left the user's config edited and unapplied with nothing said about
/// it. Asking first means a compositor that cannot be reached costs nothing.
fn reachable() -> Result<()> {
    #[cfg(unix)]
    {
        let path = socket_path()?;
        std::os::unix::net::UnixStream::connect(&path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    Err(anyhow!("Hyprland talks over a unix socket, which this platform has not got"))
}

/// Ask Hyprland something over its own command socket.
///
/// This is the whole of what `hyprctl` does. The binary cannot come along to a
/// sandbox — it links libhyprutils, libhyprwire, libre2 and libreadline, none
/// of which exist in a Flatpak runtime — but the protocol underneath is a line
/// written to a unix socket and the reply read back to EOF, with `j/` in front
/// of a command asking for JSON. That works from inside the sandbox with
/// `--filesystem=xdg-run/hypr`, and drops a subprocess from the native build.
pub(crate) fn ask(command: &str) -> Result<String> {
    #[cfg(unix)]
    {
        use std::io::{Read, Write};

        let path = socket_path()?;
        let mut stream = std::os::unix::net::UnixStream::connect(&path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        stream.write_all(command.as_bytes())?;
        // Hyprland closes its side when the reply is done, so there is no
        // length to read first. Decoded lossily: a window title can carry
        // bytes that are not UTF-8, and that is not a reason to fail.
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).with_context(|| format!("reading {command}"))?;
        Ok(String::from_utf8_lossy(&reply).to_string())
    }
    #[cfg(not(unix))]
    Err(anyhow!("Hyprland talks over a unix socket, which this platform has not got"))
}

// ---------------------------------------------------------------------------
// managed blocks
// ---------------------------------------------------------------------------

fn remove_block(body: &str) -> String {
    let Some(start) = body.find(BEGIN) else { return body.to_string() };
    let Some(rel_end) = body[start..].find(END) else {
        // Unterminated block (hand-edited or truncated) — drop to end of file.
        return body[..start].to_string();
    };
    let end = start + rel_end + END.len();
    // Also swallow the newline that followed the closing marker.
    let end = body[end..].find('\n').map(|i| end + i + 1).unwrap_or(body.len());
    format!("{}{}", &body[..start], &body[end..])
}

fn write_block(path: &Path, block: &str) -> Result<()> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let stripped = remove_block(&original);
    let mut next = stripped.trim_end().to_string();
    if !next.is_empty() {
        next.push_str("\n\n");
    }
    next.push_str(BEGIN);
    next.push('\n');
    next.push_str(block.trim_end());
    next.push('\n');
    next.push_str(END);
    next.push('\n');
    write_with_backup(path, &next)
}

fn remove_block_file(path: &Path) -> Result<()> {
    let Ok(original) = std::fs::read_to_string(path) else { return Ok(()) };
    if !original.contains(BEGIN) {
        return Ok(());
    }
    let next = remove_block(&original);
    write_with_backup(path, &next)
}

/// Back up once (never overwriting an older backup) before the first edit.
fn write_with_backup(path: &Path, content: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if path.exists() {
        let bak = path.with_extension("lua.orra.bak");
        if !bak.exists() {
            std::fs::copy(path, &bak)
                .with_context(|| format!("backing up {}", path.display()))?;
        }
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    // Setup mutates a couple of fields on `Config::default()`. Naming all forty
    // fields to satisfy the lint would bury what each test is actually varying.
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use crate::config::Provider;

    /// What a sandboxed install writes into the bind. It has to name the
    /// subcommand and the app id, because the host has no `orra-ctl` to call
    /// and no other way to reach the one inside the sandbox.
    #[test]
    fn a_sandboxed_bind_re_enters_the_flatpak() {
        assert_eq!(
            flatpak_invocation("ai.orra.desktop"),
            "flatpak run --command=orra-ctl ai.orra.desktop"
        );
    }

    /// The socket path is the whole contract with Hyprland: get it wrong and
    /// every call fails, which for `apply` means the user's keybinds silently
    /// stop being written.
    #[test]
    fn the_command_socket_is_where_hyprland_puts_it() {
        let path = socket_in(Path::new("/run/user/1000"), std::ffi::OsStr::new("abc123"));
        assert_eq!(path, PathBuf::from("/run/user/1000/hypr/abc123/.socket.sock"));
    }

    #[test]
    fn block_is_appended_once_and_replaced_on_reapply() {
        let dir = std::env::temp_dir().join(format!("orra_hypr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("keybinds.lua");
        std::fs::write(&f, "hl.bind(\"SUPER + Q\", close)\n").unwrap();

        write_block(&f, "FIRST").unwrap();
        let once = std::fs::read_to_string(&f).unwrap();
        assert!(once.contains("FIRST"));
        assert!(once.starts_with("hl.bind(\"SUPER + Q\", close)"));

        write_block(&f, "SECOND").unwrap();
        let twice = std::fs::read_to_string(&f).unwrap();
        assert!(twice.contains("SECOND"));
        assert!(!twice.contains("FIRST"), "old block must be replaced, not stacked");
        assert_eq!(twice.matches(BEGIN).count(), 1);
        // The user's own bind survives untouched.
        assert!(twice.contains("SUPER + Q"));

        remove_block_file(&f).unwrap();
        let gone = std::fs::read_to_string(&f).unwrap();
        assert!(!gone.contains(BEGIN));
        assert!(gone.contains("SUPER + Q"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn binds(cfg: &Config) -> Vec<String> {
        keybind_block(cfg)
            .lines()
            .filter(|l| l.starts_with("hl.bind("))
            .map(|l| l.to_string())
            .collect()
    }

    /// The binds for one job, so a test can count them without the others
    /// getting in the way.
    fn binds_for(cfg: &Config, job: &str) -> Vec<String> {
        binds(cfg).into_iter().filter(|l| l.contains(job)).collect()
    }

    #[test]
    fn hold_mode_writes_a_press_and_a_release_bind() {
        let mut cfg = Config::default();
        cfg.mode = Mode::Hold;
        cfg.hotkey = "SUPER + ALT + D".into();
        cfg.translate_hotkey = "SUPER + ALT + T".into();

        let dictate = binds_for(&cfg, "orra-ctl start");
        assert_eq!(dictate.len(), 1);
        assert!(dictate[0].contains("\"SUPER + ALT + D\""));
        let release = binds_for(&cfg, "release = true");
        // One release bind per held key: dictating and translating.
        assert_eq!(release.len(), 2);
        assert!(release.iter().all(|l| l.contains("orra-ctl stop")));
    }

    #[test]
    fn toggle_mode_writes_exactly_one_bind_per_key() {
        let mut cfg = Config::default();
        cfg.mode = Mode::Toggle;
        cfg.hotkey = "SUPER + ALT + D".into();
        cfg.translate_hotkey = "SUPER + ALT + T".into();

        // No release binds at all when a tap does the toggling.
        assert!(binds_for(&cfg, "release = true").is_empty());
        assert_eq!(binds_for(&cfg, "orra-ctl toggle").len(), 1);
        assert_eq!(binds_for(&cfg, "orra-ctl translate").len(), 1);
    }

    #[test]
    fn the_translating_key_gets_its_own_bind() {
        let mut cfg = Config::default();
        cfg.hotkey = "SUPER + ALT + D".into();
        cfg.translate_hotkey = "SUPER + ALT + T".into();

        let translate = binds_for(&cfg, "orra-ctl translate");
        assert_eq!(translate.len(), 1);
        assert!(translate[0].contains("\"SUPER + ALT + T\""));
    }

    #[test]
    fn no_translate_bind_without_a_key_of_its_own() {
        let mut cfg = Config::default();
        cfg.hotkey = "SUPER + ALT + D".into();

        // Empty means the feature is off...
        cfg.translate_hotkey = "  ".into();
        assert!(binds_for(&cfg, "orra-ctl translate").is_empty());

        // ...and the same key twice would only fight with itself.
        cfg.translate_hotkey = "SUPER + ALT + D".into();
        assert!(binds_for(&cfg, "orra-ctl translate").is_empty());
    }

    #[test]
    fn the_language_key_gets_its_own_bind() {
        let mut cfg = Config::default();
        cfg.translate_hotkey = String::new();
        cfg.language_hotkey = "SUPER + ALT + L".into();
        let lang: Vec<String> = binds(&cfg)
            .into_iter()
            .filter(|l| l.contains("lang-next"))
            .collect();
        assert_eq!(lang.len(), 1);
        assert!(lang[0].contains("\"SUPER + ALT + L\""));
    }

    /// The switch key is not Deepgram's: a whisper server takes a language too,
    /// and it is the only way to move between them without opening settings.
    /// Gemini, which detects the language itself, is the one that gets nothing.
    #[test]
    fn the_language_key_follows_the_provider_that_takes_a_language() {
        let mut cfg = Config::default();
        cfg.translate_hotkey = String::new();
        assert!(keybind_block(&cfg).contains("lang-next"), "Deepgram has a language");

        for provider in [Provider::Ollama, Provider::WhisperCpp, Provider::Local, Provider::Orra] {
            cfg.provider = provider;
            assert!(keybind_block(&cfg).contains("lang-next"), "{provider:?} takes a language");
        }

        cfg.provider = Provider::Gemini;
        assert!(
            !keybind_block(&cfg).contains("lang-next"),
            "Gemini detects the language and takes no code"
        );
    }

    #[test]
    fn no_language_bind_when_there_is_nothing_to_cycle() {
        let mut cfg = Config::default();
        cfg.language_cycle = vec!["en".into()];
        assert!(!keybind_block(&cfg).contains("lang-next"));

        // A key that cannot be pressed is as good as no key at all.
        cfg.language_cycle = vec!["en".into(), "fa".into()];
        cfg.language_hotkey = "   ".into();
        assert!(!keybind_block(&cfg).contains("lang-next"));
    }

    #[test]
    fn a_quote_in_a_hotkey_cannot_break_out_of_its_string() {
        // The key is free text from the settings screen. Unescaped, the quotes
        // here would close the literal early and leave the rest of the line to
        // be read as Lua.
        let mut cfg = Config::default();
        cfg.hotkey = r#"SUPER + " .. os.execute("x") .. ""#.into();
        cfg.translate_hotkey = String::new();

        let block = keybind_block(&cfg);
        // Every bind carries the key as one properly escaped literal.
        assert!(
            block.contains(&format!("hl.bind({},", lua_string(cfg.hotkey.trim()))),
            "the key was not interpolated as an escaped literal:\n{block}"
        );
        // ...and the raw form, which would end the literal early, is gone.
        assert!(
            !block.contains(&format!("hl.bind(\"{}\",", cfg.hotkey.trim())),
            "an unescaped key literal survived:\n{block}"
        );
    }

    #[test]
    fn lua_string_escapes_the_characters_that_matter() {
        assert_eq!(lua_string("SUPER + ALT + D"), r#""SUPER + ALT + D""#);
        assert_eq!(lua_string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(lua_string(r"a\b"), r#""a\\b""#);
        assert_eq!(lua_string("a\nb"), r#""a\nb""#);
    }
}
