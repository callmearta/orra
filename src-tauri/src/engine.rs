//! Running a speech-to-text model on this machine, for people who have not set
//! one up.
//!
//! Pointing Orra at a server you already run is one thing — that is
//! [`crate::local`], and it is a URL and a model name. This is the other half:
//! the app fetches the engine and the weights, starts the server itself, and
//! writes the URL it chose into the settings, so dictation works on a machine
//! with nothing installed and nobody reading a README.
//!
//! Nothing is bundled in the installer. The engine is a whisper.cpp build taken
//! from a pinned release, and the weights come from Hugging Face; both are
//! checked against a sha256 pinned here before they are used, because one of
//! them is executed and the other is loaded by it.
//!
//! Everything lands under [`config::data_dir`]: `bin/whisper/` for the engine
//! and `models/` for the weights.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, Config};
use crate::state::AppState;

/// How often the interface hears about a download in progress. Often enough to
/// look alive on a slow connection, rarely enough not to flood the webview with
/// events on a fast one.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// How long the server is given to answer after being started.
///
/// Starting it means loading the model, which for the largest one here is over
/// a minute on a cold page cache. A dictation would be lost to a shorter bound,
/// and the wait is visible as progress rather than as nothing happening.
const START_TIMEOUT: Duration = Duration::from_secs(180);

/// The whisper.cpp build the engine comes from.
///
/// A build tag rather than a version tag: the published binaries hang off the
/// builds (`b5130`), and the `vX.Y.Z` tags next to them carry no assets at all.
const BUILD_TAG: &str = "b5130";

/// The engine archive for this platform, and the hash it must have.
///
/// Pinned, not "latest": this is a binary the app downloads and then runs, so
/// what it runs has to be the thing that was reviewed. `None` where upstream
/// publishes no command-line build — macOS ships an xcframework — which is what
/// hides the whole feature there rather than offering a button that cannot work.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "whisper-bin-ubuntu-x64.tar.gz",
    "53e7fd8b5764edad916b8848dd0af6abb1ff1d3b86c899e79c78652412536c32",
));
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const ASSET: Option<(&str, &str)> = Some((
    "whisper-bin-x64.zip",
    "f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c",
));
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const ASSET: Option<(&str, &str)> = None;

/// Where the archive comes from, derived from the pinned build.
fn asset_url(asset: &str) -> String {
    format!("https://github.com/ggml-org/whisper.cpp/releases/download/{BUILD_TAG}/{asset}")
}

fn model_url(file: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}")
}

/// One model the app can fetch and run.
pub struct Model {
    /// What the interface and the settings call it. Stored in the config, so
    /// renaming one of these orphans whatever is already downloaded.
    pub name: &'static str,
    pub label: &'static str,
    /// The file on Hugging Face, which is also the file on disk.
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// What it costs the user, in the words they would use.
    pub note: &'static str,
}

/// The models on offer, cheapest first.
///
/// Multilingual rather than `.en` throughout: this app is used by people who
/// dictate in more than one language, and the English-only builds buy accuracy
/// these users cannot spend.
pub const MODELS: &[Model] = &[
    Model {
        name: "tiny",
        label: "Tiny",
        file: "ggml-tiny.bin",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        note: "The quickest to download and the roughest. Good for finding out whether this works at all.",
    },
    Model {
        name: "base",
        label: "Base",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        note: "A step up from tiny and still quick on any CPU.",
    },
    Model {
        name: "small",
        label: "Small",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        note: "The usual balance: accurate enough to keep, fast enough to leave running.",
    },
    Model {
        name: "large-v3-turbo",
        label: "Large v3 Turbo",
        file: "ggml-large-v3-turbo.bin",
        bytes: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        note: "The best of these, and the slowest on a CPU. Wants a few gigabytes of memory while it runs.",
    },
];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------------------
// paths
// ---------------------------------------------------------------------------

/// Where the engine's files are unpacked.
fn engine_dir() -> PathBuf {
    config::data_dir().join("bin").join("whisper")
}

fn models_dir() -> PathBuf {
    config::data_dir().join("models")
}

fn server_binary() -> PathBuf {
    let name = if cfg!(windows) { "whisper-server.exe" } else { "whisper-server" };
    engine_dir().join(name)
}

/// Whether the engine is unpacked and ready to start.
pub fn engine_installed() -> bool {
    server_binary().is_file()
}

/// Where a model's weights are, whether or not they are there yet.
pub fn weights(model: &Model) -> PathBuf {
    models_dir().join(model.file)
}

/// Whether a model is downloaded.
///
/// Presence is enough: the file is written to a temporary name and only renamed
/// into place once its hash has matched, so a file that exists is a file that
/// arrived whole.
pub fn installed(model: &Model) -> bool {
    weights(model).is_file()
}

/// Where the server's own output goes. Kept because the one failure worth
/// diagnosing — a model that will not load — is explained there and nowhere
/// else.
fn log_path() -> PathBuf {
    config::data_dir().join("engine.log")
}

// ---------------------------------------------------------------------------
// what the interface shows
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CatalogEntry {
    pub name: String,
    pub label: String,
    pub note: String,
    /// Rounded up, because a size is a rough idea of a wait, not a figure.
    pub mb: u64,
    pub installed: bool,
}

/// What the Settings card needs to draw itself.
#[derive(Serialize)]
pub struct Availability {
    /// False where there is no engine build to fetch, which is what hides the
    /// whole block rather than offering a button that cannot work.
    pub supported: bool,
    pub engine: bool,
    pub running: bool,
    /// The model this app is running itself; absent when the user runs their
    /// own server, which is the ordinary case.
    pub running_model: Option<String>,
    pub models: Vec<CatalogEntry>,
}

pub fn availability(app: &AppHandle) -> Availability {
    Availability {
        // No engine build to fetch on this platform is the one thing that hides
        // the whole block, rather than offering a button that cannot work.
        supported: ASSET.is_some(),
        engine: engine_installed(),
        running: running(app),
        // Which model is loaded *now*, rather than which one the settings say
        // should be: after a failed start those two disagree, and the one
        // worth showing is the one that is actually answering.
        running_model: running_model(app),
        models: MODELS
            .iter()
            .map(|m| CatalogEntry {
                name: m.name.to_string(),
                label: m.label.to_string(),
                note: m.note.to_string(),
                mb: m.bytes.div_ceil(1_000_000),
                installed: installed(m),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// downloading
// ---------------------------------------------------------------------------

/// What a download is doing, for the progress line in Settings.
#[derive(Serialize, Clone)]
struct Progress<'a> {
    /// `engine` or the model's name.
    what: &'a str,
    /// What the interface should call it.
    label: &'a str,
    received: u64,
    total: u64,
}

fn report(app: &AppHandle, what: &str, label: &str, received: u64, total: u64) {
    let _ = app.emit(
        EVT_LOCAL,
        Progress { what, label, received, total },
    );
}

/// The event the Settings card listens to while something is downloading or
/// starting, and — as [`DONE`] — when there is nothing left in flight.
pub const EVT_LOCAL: &str = "local-download";

/// The `what` of the event that says the work is over, whether it worked or not.
///
/// Without it the interface has no way to know a start has finished: it is told
/// repeatedly that one is in progress, and the last thing it heard stays on
/// screen. That is only visible when the engine was started by something other
/// than a button on that card — switching the provider to this one starts it,
/// and so does launching the app — which is exactly when nobody is watching the
/// button that would have cleared it.
pub const DONE: &str = "done";

/// Fetch `url` into `target`, hashing it as it arrives.
///
/// The hash is checked before the file is given its real name, so a truncated
/// download or a proxy's error page can never end up looking like a model. The
/// write goes to `<target>.part` for the same reason: an interrupted download
/// leaves something obviously unfinished behind rather than a file that half
/// works.
fn fetch(
    label: &str,
    url: &str,
    target: &Path,
    bytes: u64,
    sha256: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("no directory to download into"))?;
    std::fs::create_dir_all(dir).map_err(|e| anyhow!("could not create {}: {e}", dir.display()))?;

    let part = target.with_extension("part");
    let _ = std::fs::remove_file(&part);

    let resp = ureq::get(url)
        .config()
        // No overall deadline — this is a gigabyte over whatever connection the
        // user has — but a connection that stops answering should not hang the
        // download forever.
        .timeout_global(None)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .build()
        .call()
        .map_err(|e| anyhow!("could not reach {url}: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!(
            "{url} returned HTTP {} — has that release been taken down?",
            status.as_u16()
        ));
    }
    // The server's own figure, when it gives one: a download that ends early
    // can then be called what it is, rather than waiting to be caught by the
    // hash with a message about the hash.
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(bytes);

    let mut body = resp.into_body().into_reader();
    let mut file = std::fs::File::create(&part)
        .map_err(|e| anyhow!("could not write to {}: {e}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut received = 0u64;
    let mut last = Instant::now();

    loop {
        let n = body.read(&mut buf).map_err(|e| anyhow!("the download stopped early: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| anyhow!("could not write to disk: {e}"))?;
        hasher.update(&buf[..n]);
        received += n as u64;
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            progress(received, total);
        }
    }
    file.flush().map_err(|e| anyhow!("could not write to disk: {e}"))?;
    drop(file);

    // A zero total means the server never said how big it was, and there is
    // nothing to compare against — the hash below is the real check.
    if total > 0 && received != total {
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!(
            "{label} came down short — {received} of {total} bytes. Check the connection and try again."
        ));
    }

    let digest = hex(&hasher.finalize());
    if digest != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!(
            "{label} did not match the checksum it was fetched against, so it has been thrown away. \
             That means the download was tampered with in transit, or the file changed upstream — \
             in which case this build of Orra needs updating."
        ));
    }

    std::fs::rename(&part, target)
        .map_err(|e| anyhow!("could not put {} in place: {e}", target.display()))?;
    progress(received, total);
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Unpack the engine archive.
///
/// `tar` rather than a zip crate: the Linux build is a `.tar.gz` and the
/// Windows one is a `.zip`, and Windows' own `tar.exe` (bsdtar) reads both —
/// so one command covers the platforms this exists for, and neither needs a
/// dependency to read its own archive format.
fn unpack(archive: &Path, into: &Path) -> Result<()> {
    if into.exists() {
        std::fs::remove_dir_all(into).map_err(|e| anyhow!("could not clear {}: {e}", into.display()))?;
    }
    std::fs::create_dir_all(into).map_err(|e| anyhow!("could not create {}: {e}", into.display()))?;

    // `--strip-components=1` drops the archive's top-level directory, so the
    // server and the libraries beside it land where `server_binary` looks for
    // them whatever the release calls that directory.
    let out = Command::new("tar")
        .arg("xf")
        .arg(archive)
        .args(["--strip-components=1", "-C"])
        .arg(into)
        .output()
        .map_err(|e| anyhow!("could not run tar to unpack the engine: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "unpacking the engine failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let server = server_binary();
    if !server.is_file() {
        return Err(anyhow!(
            "the engine archive did not contain whisper-server — has the release layout changed?"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Fetch whatever is missing for `model` — the engine the first time, the
/// weights every time — and leave it on disk. Nothing is started.
pub fn download(app: &AppHandle, name: &str) -> Result<()> {
    let result = fetch_files(app, name);
    // Announced whatever happened. A download that failed has still stopped, and
    // a progress line left on screen with nothing behind it is worse than the
    // failure card that explains it.
    report(app, DONE, model(name).map(|m| m.label).unwrap_or(name), 0, 0);
    result
}

fn fetch_files(app: &AppHandle, name: &str) -> Result<()> {
    let asset = ASSET.ok_or_else(|| {
        anyhow!("Orra cannot fetch a local engine on this platform yet — point the Local server provider at one you run instead.")
    })?;
    let model = model(name).ok_or_else(|| anyhow!("there is no local model called {name}"))?;

    if !engine_installed() {
        let archive = config::data_dir().join("downloads").join(asset.0);
        fetch(
            "the engine",
            &asset_url(asset.0),
            &archive,
            // Not known in advance for the engine, so the server's own
            // content-length is what the progress and the short-download check
            // use; this is only the fallback.
            0,
            asset.1,
            &mut |received, total| report(app, "engine", "the engine", received, total),
        )?;
        unpack(&archive, &engine_dir())?;
        let _ = std::fs::remove_file(&archive);
    }

    if !installed(model) {
        fetch(
            model.label,
            &model_url(model.file),
            &weights(model),
            model.bytes,
            model.sha256,
            &mut |received, total| report(app, model.name, model.label, received, total),
        )?;
    }
    Ok(())
}

/// Start the engine and write the address it landed on into the settings.
///
/// The port is picked fresh every time, because the one before it may still be
/// held by a server that has only just been stopped — so what is saved goes
/// stale on every start, and a dictation would be pointed at a port with
/// nothing behind it. Whoever starts the engine owns fixing that up.
pub fn start_and_record(app: &AppHandle, model: &Model) -> Result<String> {
    let started = start(app, model);
    // The wait reported itself as it went; this says it is over. Whoever asked
    // — a button on the settings card, the settings save that switched the
    // provider to this one, or the launch that brought it up — there is nothing
    // left in flight, and the last thing the interface heard would otherwise
    // stay on screen for good.
    report(app, DONE, model.label, 0, 0);
    let url = started?;

    let saved = {
        let state = app.state::<AppState>();
        let mut cfg = state
            .cfg
            .lock()
            .map_err(|_| anyhow!("the settings lock is poisoned"))?;
        write_settings(&mut cfg, model.name, &url);
        cfg.clone()
    };
    saved.save().map_err(|e| anyhow!("could not save settings: {e}"))?;

    // What the settings screen listens to, and how its card learns the engine
    // is up without the user going back to it and asking.
    let _ = app.emit("status", ());
    Ok(url)
}

/// Start the server on a model, replacing any server this app already started.
pub fn start(app: &AppHandle, model: &Model) -> Result<String> {
    if !engine_installed() {
        return Err(anyhow!("the local engine is not downloaded yet"));
    }
    if !installed(model) {
        return Err(anyhow!("{} is not downloaded yet", model.label));
    }

    stop(app);
    let port = free_port().ok_or_else(|| anyhow!("no free port on this machine to run the engine on"))?;
    let url = format!("http://127.0.0.1:{port}/inference");

    let mut process = EngineProcess { child: server(model, port)?, model: model.name.to_string() };
    wait_until_answering(&mut process, port, &mut || report(app, "starting", "the engine", 0, 0))
        .map_err(|e| anyhow!("{e}\n\n{}", tail_of_log()))?;

    *app.state::<AppState>().engine.lock().unwrap() = Some(process);
    Ok(url)
}

/// Start the server for a model, with the flags this app always uses.
///
/// Split out from [`start`] because the flags are the part that has to be right
/// and the only way to know is to run them — which a test can do without an app
/// handle around it.
fn server(model: &Model, port: u16) -> Result<Child> {
    let log = std::fs::File::create(log_path())
        .map_err(|e| anyhow!("could not open {}: {e}", log_path().display()))?;
    let errlog = log.try_clone().map_err(|e| anyhow!("could not open the engine log: {e}"))?;

    Command::new(server_binary())
        .arg("-m")
        .arg(weights(model))
        .args(["--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        // Leaving cores for the rest of the machine: this runs while the user
        // is working, and a transcription that takes every thread makes the
        // whole desktop stutter.
        .args(["-t", &threads().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errlog))
        .spawn()
        .map_err(|e| anyhow!("could not start the engine: {e}"))
}

/// How many threads the engine may have.
fn threads() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    // Half, so a dictation does not take the machine over, and never more than
    // eight: whisper stops scaling well before that.
    (cores / 2).clamp(2, 8)
}

/// A port nothing is listening on.
///
/// Bound and released rather than kept: the engine has to bind it itself, and
/// between the two there is a window in which something else could take it —
/// which shows up as the engine failing to start, and is why this tries the
/// next one rather than insisting on the first.
fn free_port() -> Option<u16> {
    (47_812..47_832).find(|port| {
        // Bound and released: the engine has to bind it itself.
        std::net::TcpListener::bind(("127.0.0.1", *port)).map(drop).is_ok()
    })
}

/// Wait for the server to answer, which is loading the model and then listening.
fn wait_until_answering(
    process: &mut EngineProcess,
    port: u16,
    on_wait: &mut dyn FnMut(),
) -> Result<()> {
    let deadline = Instant::now() + START_TIMEOUT;
    let url = format!("http://127.0.0.1:{port}/");

    loop {
        // A server that exited has already said why, in its log.
        if let Ok(Some(status)) = process.child.try_wait() {
            return Err(anyhow!("the engine stopped as soon as it started ({status})"));
        }
        // Any answer at all counts, including an error page: what is being
        // waited for is a process that is listening, and the first thing it
        // says is not this app's business.
        if ureq::get(&url)
            .config()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .build()
            .call()
            .is_ok()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "the engine did not start within {}s",
                START_TIMEOUT.as_secs()
            ));
        }
        // There is no progress to show — the wait is inside the server loading
        // the model — so the event is only there to keep the interface from
        // looking stuck.
        on_wait();
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// The last few lines of the engine's log, for a failure that needs explaining.
fn tail_of_log() -> String {
    let Ok(body) = std::fs::read_to_string(log_path()) else {
        return "The engine wrote nothing to its log.".to_string();
    };
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    if tail.is_empty() {
        "The engine wrote nothing to its log.".to_string()
    } else {
        format!("The engine said:\n{tail}")
    }
}

/// A running engine, killed when this is dropped.
pub struct EngineProcess {
    child: Child,
    pub model: String,
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        // The app exiting is not a reason to leave a server holding a gigabyte
        // of memory, and it is not started by anything else, so it goes with
        // the app that started it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Stop any engine left behind by an earlier run of this app.
///
/// A clean quit takes the child with it, but a signal does not: killed with
/// `SIGTERM` or `SIGKILL`, or crashed, this app never reaches its destructors,
/// and the whisper server goes on holding the model in memory with nothing left
/// that knows it is there. Reaping the leftovers at startup is what keeps that
/// from accumulating a gigabyte at a time.
///
/// Only processes whose executable is *this app's* engine binary are touched —
/// the exact path under the data directory that only this code starts — so the
/// worst a mistake here can do is stop a server this app started.
#[cfg(target_os = "linux")]
fn reap_leftovers() {
    let ours = engine_dir();
    let Ok(entries) = std::fs::read_dir("/proc") else { return };

    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
        // Not readable for other users' processes, which is fine: ours are ours.
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else { continue };
        if !exe.starts_with(&ours) {
            continue;
        }
        // Safe: a pid that has since been reaped fails harmlessly, and the check
        // above is what keeps that from being someone else's process.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        println!("[orra] stopped a leftover engine (pid {pid})");
    }
}

/// Nothing to reap on the platforms that are not Linux yet. Windows would need
/// a job object to do the same thing; until there is a build to test it on,
/// this says so rather than pretending.
#[cfg(not(target_os = "linux"))]
fn reap_leftovers() {}

/// Stop the engine this app started, if any.
pub fn stop(app: &AppHandle) {
    if let Ok(mut slot) = app.state::<AppState>().engine.lock() {
        if let Some(mut process) = slot.take() {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
    }
}

pub fn running(app: &AppHandle) -> bool {
    running_model(app).is_some()
}

/// The model the engine is running, if one is.
fn running_model(app: &AppHandle) -> Option<String> {
    app.state::<AppState>()
        .engine
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|process| process.model.clone()))
}

/// Start the engine the settings say this app is responsible for.
///
/// Called at launch, in the background: bringing a server up takes as long as
/// loading its model, and nothing about the window should wait on that.
pub fn start_configured(app: AppHandle) {
    let name = app.state::<AppState>().config().local_engine_model;
    std::thread::spawn(move || {
        // Before anything else, and whether or not this run wants an engine:
        // whatever a previous run left behind is holding memory nobody is
        // accounting for.
        reap_leftovers();

        if name.is_empty() {
            return;
        }
        let Some(model) = model(&name) else {
            // The stored name is not one of ours — a downgrade, or a hand-edited
            // config. Saying so beats starting the wrong model quietly.
            crate::problem::report(
                &app,
                "Could not start the local model",
                format!("{name} is not a model this version knows about"),
            );
            return;
        };
        if let Err(e) = start_and_record(&app, model) {
            crate::problem::report(&app, "Could not start the local model", e);
        }
    });
}

/// Point the settings at the engine this app is running.
///
/// The URL and the transport are the app's business once it started the server
/// itself. The model field is cleared rather than filled in: the engine is
/// given its model at startup, and the request carries none. Whatever key was
/// in there is left alone — it belonged to a server the user was running
/// before, and quietly deleting something they typed is not this code's to do.
pub fn write_settings(cfg: &mut Config, name: &str, url: &str) {
    cfg.local_engine_model = name.to_string();
    cfg.local_base_url = url.to_string();
    cfg.local_transport = config::LocalTransport::Http;
    cfg.local_model = String::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalogue is what the config stores, so a name that does not resolve
    /// is a setting that silently stops meaning anything.
    #[test]
    fn every_model_in_the_catalogue_can_be_looked_up_by_the_name_it_stores() {
        for m in MODELS {
            assert_eq!(model(m.name).map(|m| m.file), Some(m.file), "{}", m.name);
            // A hash that is not a sha256 would fail every download at the end
            // of a gigabyte, which is the worst possible time to find out.
            assert_eq!(m.sha256.len(), 64, "{} has no sha256", m.name);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.name);
            assert!(m.bytes > 0 && m.note.len() > 20, "{} is not described", m.name);
        }
        assert!(model("no-such-model").is_none());
    }

    /// The URL is built from the pinned build, which is what makes the hash
    /// beside it mean anything.
    #[test]
    fn the_download_urls_are_the_pinned_ones() {
        assert_eq!(
            asset_url("whisper-bin-ubuntu-x64.tar.gz"),
            "https://github.com/ggml-org/whisper.cpp/releases/download/b5130/whisper-bin-ubuntu-x64.tar.gz"
        );
        assert_eq!(
            model_url("ggml-small.bin"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }

    #[test]
    fn a_hex_digest_reads_the_way_sha256_is_written() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }

    /// The engine runs on a port the app chose, so finding one has to work on a
    /// machine that is already using some of them.
    #[test]
    fn a_free_port_is_free_then_given_back() {
        let port = free_port().expect("a port to run the engine on");
        // Nothing is holding it open — the engine has to bind it itself.
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    /// Threads matter on a machine running dictation in the background: taking
    /// every core is what makes the whole desktop stutter.
    #[test]
    fn the_engine_never_takes_the_whole_machine() {
        let threads = threads();
        assert!((2..=8).contains(&threads), "got {threads}");
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        if cores > 4 {
            assert!(threads < cores, "{threads} of {cores} leaves nothing for the user");
        }
    }

    /// Everything the one-click path does, against the servers that actually
    /// serve it: the pinned URLs, the pinned hashes, the flags the engine is
    /// started with, and the request [`crate::local`] sends it. Stubs cannot
    /// check any of that, because the far end is what decides whether it is
    /// right — a hash that is one character off only fails at the end of a
    /// gigabyte, on the user's connection.
    ///
    /// Leaves the engine and the model in the data directory, which is what the
    /// button does too, so the second run is quick and the app finds them.
    ///
    /// Ignored by default — it downloads about 90 MB. Run with:
    ///   `cargo test engine::tests::the_engine_transcribes -- --ignored --nocapture`
    #[test]
    #[ignore = "downloads ~90 MB, then runs a real model"]
    fn the_engine_transcribes_what_it_is_given() {
        let model = model("tiny").expect("the catalogue has a tiny model");

        if !engine_installed() {
            let asset = ASSET.expect("this platform has an engine build");
            let archive = config::data_dir().join("downloads").join(asset.0);
            fetch("the engine", &asset_url(asset.0), &archive, 0, asset.1, &mut |_, _| {})
                .expect("the engine downloads");
            unpack(&archive, &engine_dir()).expect("the engine unpacks");
            let _ = std::fs::remove_file(&archive);
        }
        if !installed(model) {
            fetch(
                model.label,
                &model_url(model.file),
                &weights(model),
                model.bytes,
                model.sha256,
                &mut |_, _| {},
            )
            .expect("the model downloads and matches its hash");
        }

        let port = free_port().expect("a free port");
        let mut process = EngineProcess {
            child: server(model, port).expect("the engine starts"),
            model: model.name.to_string(),
        };
        wait_until_answering(&mut process, port, &mut || {})
            .unwrap_or_else(|e| panic!("{e}\n\n{}", tail_of_log()));

        // Speech to transcribe, made on this machine rather than downloaded, so
        // the test needs no key and no network beyond the two above.
        let Some(wav) = spoken_wav() else {
            println!("-- espeak is not installed, so there is nothing to transcribe");
            return;
        };

        let cfg = Config {
            provider: crate::config::Provider::Local,
            local_base_url: format!("http://127.0.0.1:{port}/inference"),
            local_transport: crate::config::LocalTransport::Http,
            ..Config::default()
        };
        let heard = crate::local::transcribe_http(&cfg, &wav).expect("the engine answers");
        println!("heard: {heard:?}");
        let lower = heard.to_lowercase();
        assert!(
            ["quick", "brown", "fox", "jumps", "lazy"].iter().any(|w| lower.contains(w)),
            "the engine answered, but not with the sentence it was given: {heard:?}"
        );
    }

    /// A WAV of a spoken sentence, made with espeak if it is installed.
    ///
    /// Resampled to the 16 kHz [`crate::local`] sends at, which is also the
    /// only rate whisper.cpp's server takes without being started to convert.
    fn spoken_wav() -> Option<Vec<u8>> {
        let out = Command::new("espeak-ng")
            .args(["-w", "/dev/stdout", "the quick brown fox jumps over the lazy dog"])
            .output()
            .or_else(|_| {
                Command::new("espeak")
                    .args(["-w", "/dev/stdout", "the quick brown fox jumps over the lazy dog"])
                    .output()
            })
            .ok()?;
        if !out.status.success() || out.stdout.len() < 44 {
            return None;
        }
        // espeak writes 22.05 kHz mono, whatever the device would have done.
        let samples = crate::deepgram::read_wav_pcm16(&out.stdout);
        let resampled = crate::audio::resample_to(&samples, 22_050, 16_000);
        Some(crate::audio::wav_bytes(&resampled, 16_000))
    }

    /// Nothing in the paths may escape the data directory: these are files the
    /// app fetches and then executes.
    #[test]
    fn everything_lives_under_the_data_directory() {
        let data = config::data_dir();
        for path in [engine_dir(), models_dir(), log_path(), weights(&MODELS[0])] {
            assert!(path.starts_with(&data), "{} is outside {}", path.display(), data.display());
        }
        assert!(server_binary().starts_with(engine_dir()));
    }
}
