use std::panic::{AssertUnwindSafe, PanicHookInfo};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
use studio_viewer::app::StudioApp;

/// The env var wgpu reads to pick its backend; when set, the user chose and we don't retry.
const WGPU_BACKEND_ENV: &str = "WGPU_BACKEND";
/// The hint printed whenever the GPU fails to start.
const GL_HINT: &str = "set WGPU_BACKEND=gl to start on OpenGL directly";
/// How long after the app is created a GPU panic still counts as a failed start. It covers the
/// first frames, where egui-wgpu configures the window's surface on the first resize.
const STARTUP_GRACE: Duration = Duration::from_secs(3);

/// One way of starting the renderer, tried in order until one opens a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attempt {
    /// wgpu with its default backends (Vulkan, Metal, DX12, then GL).
    WgpuDefault,
    /// wgpu on its OpenGL backend only; keeps GPU wires.
    WgpuGl,
    /// The glow (OpenGL) renderer; the canvas draws wires on the CPU.
    Glow,
}

impl Attempt {
    fn label(self) -> &'static str {
        match self {
            Attempt::WgpuDefault => "wgpu (default backend)",
            Attempt::WgpuGl => "wgpu on OpenGL",
            Attempt::Glow => "the OpenGL (glow) renderer",
        }
    }
}

fn main() -> eframe::Result<()> {
    studio_viewer::timing::start();
    let initial_path = std::env::args().nth(1).map(PathBuf::from);
    let backend_env = std::env::var(WGPU_BACKEND_ENV).ok();
    watch_startup_panics(backend_env.clone());

    let mut attempt = Attempt::WgpuDefault;
    loop {
        let (next, why) = match run(attempt, initial_path.clone()) {
            Ok(()) => return Ok(()),
            Err(Failure::Error(err)) => match next_attempt(&err, attempt, backend_env.as_deref()) {
                Some(next) => (next, err.to_string()),
                None => {
                    if is_gpu_error(&err) {
                        eprintln!("Studio: {} failed ({err}); {GL_HINT}.", attempt.label());
                    }
                    return Err(err);
                }
            },
            Err(Failure::Retry { next, why }) => (next, why),
            Err(Failure::Relaunch { why }) => {
                eprintln!(
                    "Studio: GPU start failed ({why}); restarting on OpenGL. \
                     Set WGPU_BACKEND=gl to start on OpenGL directly."
                );
                std::process::exit(relaunch_on_gl());
            }
        };
        eprintln!("Studio: {} failed ({why}); retrying with {}; {GL_HINT}.", attempt.label(), next.label());
        attempt = next;
    }
}

/// Why one attempt did not run.
enum Failure {
    /// eframe returned an error.
    Error(eframe::Error),
    /// The renderer panicked before the app was created; try the next rung in this process.
    Retry { next: Attempt, why: String },
    /// A GPU panic in the first frames, after the window's connection was set up; only a fresh
    /// process can start over (e.g. "The surface isn't supported by this adapter" on Wayland).
    Relaunch { why: String },
}

/// The attempt after `attempt` on the way down: wgpu default, wgpu on GL, glow.
fn next_rung(attempt: Attempt) -> Option<Attempt> {
    match attempt {
        Attempt::WgpuDefault => Some(Attempt::WgpuGl),
        Attempt::WgpuGl => Some(Attempt::Glow),
        Attempt::Glow => None,
    }
}

/// The attempt to try after `err`, or None to give up. Only GPU errors from wgpu are retried, and
/// never when the user picked a backend through `WGPU_BACKEND`.
fn next_attempt(err: &eframe::Error, attempt: Attempt, wgpu_backend_env: Option<&str>) -> Option<Attempt> {
    if wgpu_backend_env.is_some() || !matches!(err, eframe::Error::Wgpu(_)) {
        return None;
    }
    next_rung(attempt)
}

/// The attempt to try after a panic before the app was created, or None to give up. Glow starts
/// no wgpu, so its panics are not the GPU's; a backend the user picked is never second-guessed.
fn next_after_startup_panic(attempt: Attempt, wgpu_backend_env: Option<&str>) -> Option<Attempt> {
    if wgpu_backend_env.is_some() || attempt == Attempt::Glow {
        return None;
    }
    next_rung(attempt)
}

/// True for errors from the GPU stack (wgpu, glutin, OpenGL), where the GL hint helps.
fn is_gpu_error(err: &eframe::Error) -> bool {
    matches!(
        err,
        eframe::Error::Wgpu(_)
            | eframe::Error::Glutin(_)
            | eframe::Error::NoGlutinConfigs(..)
            | eframe::Error::OpenGL(_)
    )
}

/// When a panic on the main thread happened, relative to the app's creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanicPhase {
    /// Before the app was created.
    BeforeApp,
    /// Within [`STARTUP_GRACE`] of the app's creation.
    Starting,
    /// Later: the app has been running.
    Running,
}

/// What to do about a panic on the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanicAction {
    /// Try the next renderer in this process.
    InProcessRetry(Attempt),
    /// Run this executable again with `WGPU_BACKEND=gl`.
    Relaunch,
    /// Let the panic carry on unwinding.
    GiveUp,
}

/// What to do after a panic. Before the app exists any panic walks down the ladder in process.
/// A wgpu panic in the first frames of the default attempt relaunches on GL (the child has
/// `WGPU_BACKEND` set, so it never relaunches again). Everything else unwinds unchanged.
fn after_panic(attempt: Attempt, phase: PanicPhase, from_gpu: bool, wgpu_backend_env: Option<&str>) -> PanicAction {
    if wgpu_backend_env.is_some() {
        return PanicAction::GiveUp;
    }
    match phase {
        PanicPhase::BeforeApp => {
            next_after_startup_panic(attempt, wgpu_backend_env).map_or(PanicAction::GiveUp, PanicAction::InProcessRetry)
        }
        PanicPhase::Starting if from_gpu && attempt == Attempt::WgpuDefault => PanicAction::Relaunch,
        PanicPhase::Starting | PanicPhase::Running => PanicAction::GiveUp,
    }
}

/// The phase of a panic at `now` for an app created at `created_at` (None: not yet).
fn panic_phase(created_at: Option<Instant>, now: Instant) -> PanicPhase {
    match created_at {
        None => PanicPhase::BeforeApp,
        Some(created) if now.saturating_duration_since(created) <= STARTUP_GRACE => PanicPhase::Starting,
        Some(_) => PanicPhase::Running,
    }
}

/// True when a panic's source file lies in wgpu or egui-wgpu (crate folders such as
/// `wgpu-30.0.1`, `wgpu-core-…`, `egui-wgpu-0.36.2`).
fn is_gpu_panic_location(file: &str) -> bool {
    file.split(['/', '\\']).any(|part| {
        let name = part.strip_prefix("egui-").or_else(|| part.strip_prefix("egui_")).unwrap_or(part);
        name == "wgpu" || name.starts_with("wgpu-")
    })
}

/// What the panic hook knows about the attempt running on the main thread.
struct StartupWatch {
    attempt: Attempt,
    wgpu_backend_env: Option<String>,
    created_at: Option<Instant>,
    /// The last main-thread panic: what to do about it, and one line naming it.
    panic: Option<(PanicAction, String)>,
}

static WATCH: Mutex<StartupWatch> =
    Mutex::new(StartupWatch { attempt: Attempt::WgpuDefault, wgpu_backend_env: None, created_at: None, panic: None });

fn watch() -> std::sync::MutexGuard<'static, StartupWatch> {
    WATCH.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs a panic hook that decides, for main-thread panics, whether the start can be retried.
/// Those it can retry stay quiet (main prints one line naming them); all others print as usual.
fn watch_startup_panics(wgpu_backend_env: Option<String>) {
    watch().wgpu_backend_env = wgpu_backend_env;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() != Some("main") {
            return default_hook(info);
        }
        let action = {
            let mut watch = watch();
            let phase = panic_phase(watch.created_at, Instant::now());
            let from_gpu = info.location().is_some_and(|at| is_gpu_panic_location(at.file()));
            let action = after_panic(watch.attempt, phase, from_gpu, watch.wgpu_backend_env.as_deref());
            watch.panic = Some((action, describe_panic(info)));
            action
        };
        if action == PanicAction::GiveUp {
            default_hook(info);
        }
    }));
}

/// "panic at file:line: message", in one line.
fn describe_panic(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic");
    let message = message.lines().next().unwrap_or_default();
    match info.location() {
        Some(at) => format!("panic at {}:{}: {message}", at.file(), at.line()),
        None => format!("panic: {message}"),
    }
}

/// Opens the window with one renderer attempt. eframe reuses its event loop between calls, so a
/// failed attempt can be followed by another in the same process. A panic the hook gave up on
/// carries on unwinding.
fn run(attempt: Attempt, initial_path: Option<PathBuf>) -> Result<(), Failure> {
    {
        let mut watch = watch();
        watch.attempt = attempt;
        watch.created_at = None;
        watch.panic = None;
    }
    let creator: eframe::AppCreator<'_> = Box::new(move |cc| {
        watch().created_at = Some(Instant::now());
        Ok(Box::new(StudioApp::new(cc, initial_path)))
    });
    // The app name also names the settings directory (e.g. ~/.local/share/tbd-studio).
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        eframe::run_native("tbd-studio", native_options(attempt), creator)
    }));
    let panic = match result {
        Ok(result) => return result.map_err(Failure::Error),
        Err(panic) => panic,
    };
    match watch().panic.take() {
        Some((PanicAction::InProcessRetry(next), why)) => Err(Failure::Retry { next, why }),
        Some((PanicAction::Relaunch, why)) => Err(Failure::Relaunch { why }),
        Some((PanicAction::GiveUp, _)) | None => std::panic::resume_unwind(panic),
    }
}

/// Runs this executable again, same arguments, with `WGPU_BACKEND=gl`, and returns its exit code.
/// The app relaunching itself at startup is not a project command, so it skips `exec.rs`.
/// On Unix the process image is replaced (same pid, so `timeout` and signals reach it, and this
/// process's threads and window go away); elsewhere the child is waited for.
fn relaunch_on_gl() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            eprintln!("Studio: could not find this executable to restart it ({err}).");
            return 1;
        }
    };
    let mut command = std::process::Command::new(exe);
    command.args(std::env::args_os().skip(1)).env(WGPU_BACKEND_ENV, "gl");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        eprintln!("Studio: could not restart on OpenGL ({err}).");
        1
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("Studio: could not restart on OpenGL ({err}).");
            1
        }
    }
}

fn native_options(attempt: Attempt) -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([800.0, 600.0])
        .with_title("Studio")
        .with_app_id("tbd-studio");
    // The app draws its own title bar (app::title_bar). On macOS the native window buttons stay,
    // over the bar's left end; elsewhere the app draws its own (app::window_frame).
    let viewport = if cfg!(target_os = "macos") {
        viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        viewport.with_decorations(false)
    };
    let (renderer, wgpu_options) = match attempt {
        Attempt::WgpuDefault => (eframe::Renderer::Wgpu, WgpuConfiguration::default()),
        Attempt::WgpuGl => (eframe::Renderer::Wgpu, gl_only_wgpu()),
        Attempt::Glow => (eframe::Renderer::Glow, WgpuConfiguration::default()),
    };
    eframe::NativeOptions { viewport, renderer, wgpu_options, ..Default::default() }
}

/// A wgpu setup limited to the OpenGL backend.
fn gl_only_wgpu() -> WgpuConfiguration {
    let mut create_new = WgpuSetupCreateNew::without_display_handle();
    create_new.instance_descriptor.backends = wgpu::Backends::GL;
    WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(create_new), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui_wgpu::WgpuError;

    fn gpu_err() -> eframe::Error {
        eframe::Error::Wgpu(WgpuError::NoSurfaceFormatsAvailable)
    }

    fn app_err() -> eframe::Error {
        eframe::Error::AppCreation(Box::from("app failed"))
    }

    #[test]
    fn wgpu_default_failure_retries_on_wgpu_gl() {
        assert_eq!(next_attempt(&gpu_err(), Attempt::WgpuDefault, None), Some(Attempt::WgpuGl));
    }

    #[test]
    fn wgpu_gl_failure_retries_on_glow() {
        assert_eq!(next_attempt(&gpu_err(), Attempt::WgpuGl, None), Some(Attempt::Glow));
    }

    #[test]
    fn glow_failure_gives_up() {
        assert_eq!(next_attempt(&gpu_err(), Attempt::Glow, None), None);
    }

    #[test]
    fn non_gpu_error_gives_up_at_every_attempt() {
        for attempt in [Attempt::WgpuDefault, Attempt::WgpuGl, Attempt::Glow] {
            assert_eq!(next_attempt(&app_err(), attempt, None), None, "{attempt:?}");
        }
    }

    #[test]
    fn wgpu_backend_env_set_gives_up() {
        for attempt in [Attempt::WgpuDefault, Attempt::WgpuGl, Attempt::Glow] {
            assert_eq!(next_attempt(&gpu_err(), attempt, Some("vulkan")), None, "{attempt:?}");
            assert_eq!(next_attempt(&gpu_err(), attempt, Some("")), None, "{attempt:?}");
        }
    }

    #[test]
    fn a_startup_panic_walks_down_to_glow_unless_the_user_chose() {
        assert_eq!(next_after_startup_panic(Attempt::WgpuDefault, None), Some(Attempt::WgpuGl));
        assert_eq!(next_after_startup_panic(Attempt::WgpuGl, None), Some(Attempt::Glow));
        assert_eq!(next_after_startup_panic(Attempt::Glow, None), None);
        assert_eq!(next_after_startup_panic(Attempt::WgpuDefault, Some("gl")), None);
    }

    #[test]
    fn a_panic_before_the_app_walks_the_ladder_in_process() {
        use PanicAction::*;
        for from_gpu in [true, false] {
            let at = |attempt| after_panic(attempt, PanicPhase::BeforeApp, from_gpu, None);
            assert_eq!(at(Attempt::WgpuDefault), InProcessRetry(Attempt::WgpuGl));
            assert_eq!(at(Attempt::WgpuGl), InProcessRetry(Attempt::Glow));
            assert_eq!(at(Attempt::Glow), GiveUp);
        }
    }

    #[test]
    fn a_gpu_panic_in_the_first_frames_relaunches_on_gl() {
        assert_eq!(after_panic(Attempt::WgpuDefault, PanicPhase::Starting, true, None), PanicAction::Relaunch);
    }

    #[test]
    fn a_starting_panic_relaunches_only_from_wgpu_default() {
        // From wgpu on GL the child would start the same way; glow has no wgpu.
        for attempt in [Attempt::WgpuGl, Attempt::Glow] {
            assert_eq!(after_panic(attempt, PanicPhase::Starting, true, None), PanicAction::GiveUp, "{attempt:?}");
        }
    }

    #[test]
    fn a_non_gpu_or_late_panic_unwinds_unchanged() {
        for attempt in [Attempt::WgpuDefault, Attempt::WgpuGl, Attempt::Glow] {
            assert_eq!(after_panic(attempt, PanicPhase::Starting, false, None), PanicAction::GiveUp);
            assert_eq!(after_panic(attempt, PanicPhase::Running, true, None), PanicAction::GiveUp);
            assert_eq!(after_panic(attempt, PanicPhase::Running, false, None), PanicAction::GiveUp);
        }
    }

    #[test]
    fn wgpu_backend_env_set_never_retries_or_relaunches() {
        for attempt in [Attempt::WgpuDefault, Attempt::WgpuGl, Attempt::Glow] {
            for phase in [PanicPhase::BeforeApp, PanicPhase::Starting, PanicPhase::Running] {
                for env in ["gl", ""] {
                    assert_eq!(
                        after_panic(attempt, phase, true, Some(env)),
                        PanicAction::GiveUp,
                        "{attempt:?} {phase:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn panic_phase_follows_the_grace_window() {
        let created = Instant::now();
        assert_eq!(panic_phase(None, created), PanicPhase::BeforeApp);
        assert_eq!(panic_phase(Some(created), created), PanicPhase::Starting);
        assert_eq!(panic_phase(Some(created), created + STARTUP_GRACE), PanicPhase::Starting);
        let late = created + STARTUP_GRACE + Duration::from_millis(1);
        assert_eq!(panic_phase(Some(created), late), PanicPhase::Running);
    }

    #[test]
    fn gpu_panic_locations_are_wgpu_and_egui_wgpu_sources() {
        let registry = "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f";
        assert!(is_gpu_panic_location(&format!("{registry}/egui-wgpu-0.36.2/src/winit.rs")));
        assert!(is_gpu_panic_location(&format!("{registry}/wgpu-30.0.1/src/backend/wgpu_core.rs")));
        assert!(is_gpu_panic_location(&format!("{registry}/wgpu-core-30.0.1/src/device/mod.rs")));
        assert!(is_gpu_panic_location(r"C:\cargo\registry\src\egui-wgpu-0.36.2\src\winit.rs"));
        assert!(!is_gpu_panic_location(&format!("{registry}/egui-0.36.2/src/context.rs")));
        assert!(!is_gpu_panic_location(&format!("{registry}/egui_glow-0.36.2/src/painter.rs")));
        assert!(!is_gpu_panic_location("crates/studio_viewer/src/app/mod.rs"));
        assert!(!is_gpu_panic_location("crates/studio_canvas/src/wgpu_wires.rs"));
    }

    #[test]
    fn gpu_errors_get_the_hint_app_errors_do_not() {
        assert!(is_gpu_error(&gpu_err()));
        assert!(!is_gpu_error(&app_err()));
    }

    #[test]
    fn gl_only_setup_limits_backends_to_gl() {
        let WgpuSetup::CreateNew(create_new) = gl_only_wgpu().wgpu_setup else { panic!("expected CreateNew") };
        assert_eq!(create_new.instance_descriptor.backends, wgpu::Backends::GL);
        assert_eq!(native_options(Attempt::WgpuGl).renderer, eframe::Renderer::Wgpu);
        assert_eq!(native_options(Attempt::Glow).renderer, eframe::Renderer::Glow);
    }
}
