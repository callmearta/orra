<div align="center">
  <img src="orra-icon.png" alt="Orra" width="128" height="128">
  <h1>Orra</h1>
  <p><strong>Hold a key, speak, let go — the words are typed into whatever window has focus.</strong></p>
</div>

A desktop dictation app. It streams speech-to-text from Deepgram, AssemblyAI or
Gemini, can translate before typing, and reads text back with Deepgram Aura —
wrapped in [Tauri](https://tauri.app) so one Rust codebase builds for Linux,
macOS and Windows.

- **Streaming, not batch.** Words appear in an overlay while you speak; the
  transcript is typed the moment you release the key.
- **Three interchangeable providers.** Pick on cost, accuracy and the shape of
  the transcript.
- **Local by default.** Settings and history never leave your machine. Only the
  audio you dictate goes to the provider you chose — see
  [Privacy](#privacy-and-what-leaves-your-machine).

---

## Contents

- [Platform support](#platform-support)
- [Install](#install)
- [API keys](#api-keys)
- [Using it](#using-it)
- [Hotkeys](#hotkeys)
- [Where things live](#where-things-live)
- [Building from source](#building-from-source)
- [Tests](#tests)
- [Architecture](#architecture)
- [Troubleshooting](#troubleshooting)
- [Privacy and what leaves your machine](#privacy-and-what-leaves-your-machine)
- [Contributing](#contributing)
- [License](#license)

---

## Platform support

| | Linux (Hyprland) | Linux (X11 / other Wayland) | macOS | Windows |
|---|---|---|---|---|
| Dictation, translation, read-aloud | ✅ | ✅ | ⚠️ builds, untested | ⚠️ builds, untested |
| Global hotkey | ✅ via compositor | ⚠️ bind it yourself | ✅ via plugin | ✅ via plugin |
| Overlay positioned above the dock | ✅ | ➖ appears centred | ➖ | ➖ |
| Clipboard / keystroke injection | ✅ `wtype` | ✅ `xdotool` or `wtype` | ⚠️ `osascript` | ⚠️ PowerShell |
| Launch at login | ✅ XDG autostart | ✅ XDG autostart | ❌ not implemented | ❌ not implemented |
| Released as | ✅ AppImage, deb, rpm | ✅ AppImage, deb, rpm | ❌ not built | ✅ NSIS `.exe` |

**Linux is the target that is actually exercised.** Hyprland is first-class: the
app writes its own keybinds and window rules. Windows builds and ships an
installer, but the project has not been run on it — the clipboard and keystroke
paths are written but unverified. macOS is not built at all yet. Treat both as a
starting point rather than a supported build — see
[Porting notes](#porting-notes-macos-and-windows).

---

## Install

### From a release

Every tagged release carries a bundle for each platform — Linux AppImage, deb
and rpm, and a Windows installer. Grab the one for your system from
[Releases](https://github.com/callmearta/orra/releases).

**Debian / Ubuntu**

```bash
sudo apt install ./Orra_0.1.1_amd64.deb
```

**Fedora / RHEL**

```bash
sudo dnf install ./Orra-0.1.1-1.x86_64.rpm
```

**Any Linux, no install**

```bash
chmod +x Orra_0.1.1_amd64.AppImage
./Orra_0.1.1_amd64.AppImage
```

**Windows** — run the `Orra_0.1.1_x64-setup.exe` installer.

The deb and rpm declare `wtype` and `wl-clipboard` as dependencies, so your
package manager should pull them in. On Arch, `sudo pacman -S wtype wl-clipboard`;
on an AppImage, install them yourself. On an X11 session you want `xdotool`
instead of `wtype`.

### From source

See [Building from source](#building-from-source).

---

## API keys

You need **one** key for the provider that transcribes, and a **Deepgram** key
if you also want read-aloud (that is the only service whose voices Orra
uses). Translating needs a **Gemini** key as well, unless you point Translation
at your own OpenAI-compatible endpoint.

| Provider | Where to get a key | Free tier |
|---|---|---|
| Deepgram | <https://console.deepgram.com> | Yes, $200 credit |
| AssemblyAI | <https://www.assemblyai.com/dashboard> | Yes, limited hours |
| Google Gemini | <https://aistudio.google.com/apikey> | Yes, rate-limited |

There are three ways to supply a key. **Resolution order is: environment → a
nearby `.env` → the value saved in Settings**, so an exported variable always
wins over anything pasted into the UI.

**1. A `.env` file at the repo root** (or next to the binary). This is what the
tests read, and the easiest for a source build:

```bash
# /path/to/orra/.env
DEEPGRAM_API_KEY=your_key_here
ASSEMBLYAI_API_KEY=your_key_here
GEMINI_API_KEY=your_key_here
```

Only the key for the provider you are actually using has to be present. The file
is in `.gitignore` — **never commit it.**

**2. Environment variables**, for a shell or a launcher:

```bash
export DEEPGRAM_API_KEY=your_key_here
```

**3. Settings → paste it into the box for the provider.** This writes it in
plain text to `~/.config/orra/config.json`, so prefer one of the other two if
your home directory is backed up or synced somewhere you would not put a
credential. Click **Verify key** to check it against the provider.

---

## Using it

### Dictating

**Hold `SUPER + ALT + D`** (configurable) and speak. A rising tone marks the
start; a falling tone the end. Release, and the transcript is pasted into
whatever window has focus.

The overlay near the bottom of the screen shows the transcript live. Releasing
the key takes the overlay away at once — the audio is still being flushed to the
provider and the text has not been typed yet, which is what the falling tone
marks.

Press `SUPER + ALT + L` to step to the next dictation language without opening
Settings.

### Translation

Hold the translating key (default `SUPER + ALT + T`) and speak. The dictation is
transcribed as usual, translated, and *that* is typed. Set the target language
and service under **Settings → Translation**.

Translation runs on Gemini by default, so it needs a Gemini key whichever
provider is transcribing. It can also run against **any OpenAI-compatible
endpoint** — OpenAI, OpenRouter, Groq, or a model on your own machine — by
setting the service to *Custom endpoint* and giving it an API URL, a model name
and a key (a local server usually wants no key). The URL is the base one,
ending at `/v1`; `/chat/completions` is appended to it. **Test translation**
sends one short phrase through whatever is configured — including Gemini, which
is the only way to check a Gemini key that is used for translating and not for
transcribing — so a wrong key, model or URL is found in Settings rather than
mid-dictation.

Translation is one extra round trip after you stop speaking, which is why it
has a key of its own rather than being something every dictation waits for.
If it fails, the words you actually said are typed instead and the reason is
shown. The history entry keeps both: the translation as the dictation, and the
original underneath it.

### When something fails

A failure is shown as a card at the top of the window, and it stays there until
you close it — the failures that matter happen while you are looking at another
window, so it is not something to miss in four seconds.

It leads with what kind of failure it was (`Could not reach the service`, `The
API key was refused`, `Not set up yet`) and what to do about it, then the error
itself. **Copy log** puts all of that plus the version and the platform on the
clipboard, which is what to send with a bug report. API keys are stripped out
of it before it is shown or copied, so it is safe to paste.

### Read aloud

**Read aloud** speaks the last transcript, or any entry in the history, using
Deepgram Aura (`aura-2-thalia-en` by default). This always needs a Deepgram key,
whichever provider transcribes. Change the voice under **Voice → Speaking**.

### Voice commands

Say these while dictating. Toggle the whole feature under **Dictionary &
Formatting → Formatting → Voice commands**.

| Say | Get |
|---|---|
| `new line` | a line break |
| `new paragraph` | a blank line |
| `press enter`, `hit enter`, `send it`, `submit that` | the text is submitted |
| `question mark` / `exclamation mark` / `exclamation point` / `full stop` | `?` / `!` / `!` / `.` |
| `open paren` / `close paren` (or `open parenthesis` / `close parenthesis`) | `(` / `)` |
| `semicolon` / `hyphen` | `;` / `-` |
| `scratch that`, `delete that`, `cancel that`, `never mind that` | the previous dictation is deleted |

Any of them can be marked with a spoken `slash` — `slash new line`, or `/new
line` if that is how the transcriber heard it — which is worth doing for the
ones that are ordinary English too, like `new line` in "add a new line of
products". The marker is not required; it only makes the intent explicit.

Matching ignores the punctuation the transcriber adds, because a spoken command
is not normal speech and gets punctuated as if it were: `new line` arrives as
`new line,`, `new line.` or `new. Line,` depending on where smart formatting
decided the sentence ended. The punctuation around a command is taken with it
when it is replaced, so the comma that followed the command does not surface at
the head of the new line.

Deliberately absent: `period`, `comma`, `colon`, `dash`. They are ordinary nouns,
and rewriting them mid-sentence does more harm than good. Add them under
**Snippets & Shortcuts → Rules** if you want them.

**Filler removal** (the **Remove filler words** toggle beside it) drops standalone `um`, `uh`,
`hmm` and friends — `um, umm, uhm, uh, uhh, erm, hmm, mmm, mhm, hm` — along with
any comma left hanging off them.

### Words

- **Dictionary** — names and jargon, sent to Deepgram as keyterms so nova-3
  expects them. Deepgram only; the other providers take no keyterms.
- **Snippets & Shortcuts → Rules** — applied after transcription,
  case-insensitively and on word boundaries. Doubles as text expansion: map
  `my signature` to your full sign-off.

### History and Insights

**Transcripts & Notes** keeps the last 5,000 dictations in
`~/.config/orra/history.jsonl`. Copy, re-insert, read aloud or delete any of
them. **Insights** aggregates over the same file — words per minute, streak,
where you dictate, vocabulary breadth.

---

## Hotkeys

### Hyprland

Wayland has no global-shortcut protocol, so on Hyprland the keys are registered
by the compositor rather than the app. Pressing **Apply** in Settings writes a
delimited block to:

- `~/.config/hypr/config/keybinds.lua` — the dictate, translate and language binds
- `~/.config/hypr/config/settings.lua` — the overlay's window rule

then runs `hyprctl reload`. The block is idempotent and replaced in place, never
duplicated, and both files are backed up once to `*.lua.orra.bak` before the
first edit. Delete the lines between the `orra (managed block)` markers to
remove it by hand.

> **Note:** this targets a **Lua** Hyprland config
> (`~/.config/hypr/config/*.lua`). If yours is the older `hyprland.conf` format,
> the app cannot write your binds and `hyprctl reload` will report an error —
> bind `orra-ctl` yourself as described below.

The binds call `orra-ctl`, which talks to the running app over a loopback
socket; spawning a whole Tauri process per keypress would be far too slow. The
port and a shared token live in `~/.config/orra/config.json`.

### Any other Linux setup

`orra-ctl` is a standalone binary and works on any compositor. Bind it
yourself — in Sway, for example:

```
# ~/.config/sway/config
bindsym --release $mod+Mod1+d exec orra-ctl toggle
bindsym $mod+Mod1+t exec orra-ctl translate
bindsym $mod+Mod1+l exec orra-ctl lang-next
```

Its commands:

| Command | Effect |
|---|---|
| `start` / `stop` | begin or end a dictation |
| `toggle` | start if idle, stop if recording |
| `translate` | a dictation that is translated before typing |
| `lang-next` / `lang-prev` | step the dictation language |
| `lang <code>` | switch to a specific code, e.g. `orra-ctl lang fa` |
| `speak [text]` | read text aloud, or the last transcript if given none (also reads stdin) |
| `open` | raise the settings window |
| `status` | print `recording=true` or `recording=false` |
| `quit` | exit Orra |

For a hold-to-talk bind you need both edges:

```
bindsym --release $mod+Mod1+d exec orra-ctl stop
bindsym $mod+Mod1+d exec orra-ctl start
```

`orra-ctl` is installed next to the app binary. If you run from a source
build it is in `src-tauri/target/release/`, so use the full path in your bind.

### macOS and Windows

`tauri-plugin-global-shortcut` registers the keys in-process, so a hold bind
works the same way it does on Hyprland. macOS will ask for Accessibility
permission the first time text is injected through System Events.

---

## Where things live

| What | Path |
|---|---|
| Settings | `$XDG_CONFIG_HOME/orra/config.json` (or `~/.config/orra/config.json`) |
| History | `$XDG_CONFIG_HOME/orra/history.jsonl` |
| Autostart entry | `$XDG_CONFIG_HOME/autostart/orra.desktop` (when enabled) |
| Hyprland backup | `~/.config/hypr/config/*.lua.orra.bak` |

Deleting the `orra` config directory resets everything.

---

## Building from source

### Prerequisites

| | |
|---|---|
| Rust | 1.85 or newer (the crate is edition 2024) |
| Node.js | 20 or newer, for the frontend |
| Tauri CLI | `cargo install tauri-cli --version '^2'` (only for `cargo tauri`) |

**Linux** also needs the WebKitGTK stack and two small input tools:

```bash
# Arch
sudo pacman -S webkit2gtk-4.1 base-devel wtype wl-clipboard

# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libasound2-dev libayatana-appindicator3-dev librsvg2-dev \
  wtype wl-clipboard
```

`libasound2-dev` is the one people miss: `cpal` links ALSA to capture the
microphone, and without it the build fails in `alsa-sys` with
`Package alsa was not found in the pkg-config search path`.

For an X11 session, swap `wtype` for `xdotool`. For Wayland compositors that do
not expose the virtual-keyboard protocol `wtype` needs, `ydotool` is used as a
fallback for the paste keystroke — though not for typing text or for Enter.

**macOS**: Xcode command line tools. **Windows**: the MSVC toolchain and
WebView2.

### Build

```bash
# 1. Frontend — the Rust crate embeds the built output at compile time,
#    so this has to run before cargo does.
cd flow-insights-dashboard
npm ci
npm run build

# 2. App
cd ../src-tauri
cargo run                  # debug build; the UI is embedded, so it runs on its own
cargo build --release      # optimised binary
cargo tauri build          # release bundle (adds src-tauri/target/release/bundle/)
cargo tauri dev            # dev server + hot reload for UI edits
```

`cargo run` needs neither the Tauri CLI nor a dev server: the `custom-protocol`
feature is on by default, which embeds the frontend and serves it over
`tauri://`. Without it a plain build bakes in the URL of `tauri dev`'s static
server and shows a 404 unless that exact server happens to be the one running.
`tauri dev` is the only build that wants the server, and the CLI strips the
feature back out for it.

`build.rs` watches `flow-insights-dashboard/dist`, because those assets are baked
in when the Rust crate compiles and cargo cannot see the dependency on its own —
without it, a rebuilt frontend leaves the previous bundle embedded and the app
serves a UI that is no longer on disk.

**The app is single-instance:** starting it while a copy already runs raises that
window and exits. So a `cargo run` that appears to do nothing means an older
build is still alive (`pkill -x orra`).

`tauri.conf.json` asks for a `.deb` by default, which keeps a local
`cargo tauri build` quick. Ask for the others explicitly when you want them:

```bash
cargo tauri build --bundles appimage,deb,rpm
```

The AppImage needs `patchelf` and `libfuse2`; the rpm needs `rpm`.

### Porting notes (macOS and Windows)

Windows is built and released by CI (`.github/workflows/release.yml`, an NSIS
`.exe`). macOS is not built — to add it, change the matrix there and set
`src-tauri/tauri.conf.json`:

```jsonc
"bundle": {
  "targets": ["deb"],   // change to "all", or to "dmg"/"app"/"nsis"/"msi"
  ...
}
```

Launch-at-login is Linux-only: `set_launch_at_login` in `src-tauri/src/commands.rs`
returns an error elsewhere and needs a LaunchAgent on macOS and a registry key on
Windows. The overlay is positioned by a Hyprland window rule, so on other
platforms it appears wherever the toolkit centres it.

### Releasing

Pushing a tag matching the version in `src-tauri/tauri.conf.json` builds every
platform and publishes them to one GitHub Release:

```bash
git tag v0.1.1 && git push origin v0.1.1
```

Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and
`flow-insights-dashboard/package.json` first — the workflow refuses to build if
the tag and the app version disagree. Running the workflow by hand from the
Actions tab builds the same bundles without publishing anything.

---

## Tests

```bash
# Rust — offline unit tests, no keys or network needed
cd src-tauri && cargo test

# Live round trips against the real APIs (needs keys in .env or the environment)
cargo test -- --ignored --nocapture

# Frontend
cd flow-insights-dashboard && npm test

# Lints
cd src-tauri && cargo clippy --all-targets
```

The ignored tests synthesize a sentence with Aura, stream it back through the
listen socket, and assert it comes back as words — a genuine end-to-end check of
both endpoints, including that `multi` transcribes correctly and that
`detect_language` is never sent (the streaming endpoint rejects it).

---

## Architecture

```
src-tauri/src/
  main.rs        app setup, tray, HUD window, and the loopback control socket
  commands.rs    the #[tauri::command] surface the settings UI calls
  state.rs       shared state, the dictation lifecycle, history persistence
  stt.rs         one streaming session loop for every provider
  deepgram.rs    ┐
  assemblyai.rs  ├ each provider's URL, framing and message parser
  gemini.rs      ┘
  audio.rs       microphone capture (cpal) and playback (rodio)
  polish.rs      transcript → typed text: fillers, voice commands, replacements
  translate.rs   translation: Gemini, or any OpenAI-compatible endpoint
  problem.rs     a failure as the user sees it: kind, advice, and the log to send
  inject.rs      clipboard and keystroke delivery, per platform
  hotkeys.rs     global shortcuts; defers to hypr.rs on Hyprland
  hypr.rs        the managed Lua blocks and the overlay's window rule
  config.rs      settings persistence and key resolution
  stats.rs       the Insights aggregates
  bin/ctl.rs     orra-ctl — the tiny client the compositor binds call

flow-insights-dashboard/   React + TypeScript + Vite + Tailwind settings UI
  src/pages/               one file per screen
  src/lib/stats.ts         presentation helpers (the arithmetic lives in Rust)
```

Two design choices worth knowing before changing things:

- **One session loop, three providers.** `stt.rs` owns the connect → feed →
  drain → flush state machine. Everything provider-specific is behind the `Wire`
  struct: the URL, how audio is framed, how a server message is read, what closes
  the stream. Adding a provider means writing one module, not another loop.
- **The arithmetic is in Rust.** Streaks, words-per-minute and month boundaries
  live in `stats.rs` so `cargo test` covers them; the frontend only lays out what
  that returns.

---

## Troubleshooting

**Nothing happens when I press the hotkey.** On Hyprland, open Settings and press
**Apply** — the bind is written to your compositor config, not registered by the
app. Check `hyprctl configerrors`. On a non-Lua Hyprland config, or any other
compositor, bind `orra-ctl` yourself (see [Hotkeys](#hotkeys)).

**`orra-ctl` says "orra is not running".** The app is not up, or it could
not bind its control port. Look for `cannot listen on 127.0.0.1:<port>` on
stderr — usually a second copy already running.

**"no key injection tool found (install wtype)".** Install `wtype` (Wayland) or
`xdotool` (X11), and `wl-clipboard` for the clipboard path.

**Text arrives in the wrong application, or not at all.** Injection puts the text
on the clipboard, synthesises Ctrl+V (Ctrl+Shift+V in terminals, detected by
window class), then restores your clipboard. If your terminal is missing from the
list in `src-tauri/src/inject.rs`, add it — or switch to **Type it out** in
Settings for apps that mangle paste.

**The window is blank, or the app dies the moment it opens (NVIDIA).** WebKit's
DMABUF renderer is broken on the NVIDIA driver. Orra sets
`WEBKIT_DISABLE_DMABUF_RENDERER=1` automatically when it finds the driver loaded,
and leaves it alone otherwise since the fallback renderer is slower. Set that
variable yourself to override the decision either way.

**The overlay appears in the middle of the screen.** That is expected outside
Hyprland. A Wayland client cannot position itself, and the window rule that does
it on Hyprland has no equivalent elsewhere.

**Persian (or another language) transcribes as English.** Deepgram's
`detect_language` is batch-only — the streaming endpoint rejects it with
`400 Bad Request: Language detection is only supported for batch.` The default is
therefore nova-3's `language=multi`, which follows switching between the ten
languages it covers (English, Spanish, French, German, Hindi, Russian,
Portuguese, Japanese, Italian, Dutch). Anything outside that set needs its code
selected explicitly — press the language key, or add codes under **Voice →
Languages to switch between**. AssemblyAI streaming covers 18 languages and
silently *ignores* an unsupported code, so Orra only sends the ones it
supports. Gemini detects the language itself and takes no codes.

**`cargo build` fails with a missing `dist`.** The frontend has to be built
first — see [Building from source](#building-from-source).

---

## Privacy and what leaves your machine

- **Audio** goes only to the provider you configured, over TLS, while you hold
  the key.
- **Transcripts** are sent to that provider for the duration of the stream, to
  Deepgram a second time if you use read-aloud, and to the translation service
  — Gemini, or the endpoint you configured — if you use translation.
- **Settings and history never leave your machine.** They are plain files under
  `~/.config/orra/` — no telemetry, no analytics, no accounts.
- **API keys** are read from the environment or a `.env`, or stored in plain text
  in `config.json` if you paste them into Settings. `.env` is gitignored; keep it
  that way. Keys are stripped out of the error log that **Copy log** puts on the
  clipboard, because that text is meant to be sent to someone.
- **The control socket** listens on `127.0.0.1` only and requires a shared token,
  generated on first run into `config.json`, so unrelated local processes cannot
  make the app type.

Dictation history is not encrypted. If that matters to you, keep the config
directory on an encrypted volume and delete entries you do not want to keep.

---

## Contributing

Issues and pull requests are welcome. Before opening a PR:

```bash
cd src-tauri && cargo test && cargo clippy --all-targets
cd ../flow-insights-dashboard && npm run build && npm test
```

Both should be clean. The code is commented to explain *why* rather than *what* —
please keep that up. The tests are the specification for the fiddly parts
(voice-command matching, the transcript-to-text rules, the Insights arithmetic),
so add one alongside a change there.

---

## License

No license is granted yet — all rights reserved. If you want to use this code,
please open an issue.
