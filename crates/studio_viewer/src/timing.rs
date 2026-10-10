//! Start-up timings for the stage gate. With `STUDIO_TIMINGS=1` the viewer prints one line per
//! event to stderr, measured from process start, e.g. `[timing] +   812 ms first layout shown`.
//! Off by default: the variable is read once, and each event then costs one atomic load.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use studio_sources::SourceState;

/// Process start when timings are on, `None` when off.
static START: OnceLock<Option<Instant>> = OnceLock::new();
/// Whether the first layout has been reported.
static LAYOUT_SHOWN: AtomicBool = AtomicBool::new(false);

/// Takes the start instant and reads `STUDIO_TIMINGS`. Call first thing in `main`.
pub fn start() {
    START.get_or_init(|| (std::env::var_os("STUDIO_TIMINGS").is_some_and(|v| v == "1")).then(Instant::now));
}

/// Whether timings are printed.
pub fn enabled() -> bool {
    matches!(START.get(), Some(Some(_)))
}

/// Prints `what` with the time since start, when timings are on; `what` is only built then.
pub fn event(what: impl FnOnce() -> String) {
    if let Some(Some(start)) = START.get() {
        eprintln!("{}", line(start.elapsed(), &what()));
    }
}

/// Reports the first layout on screen, once per process.
pub fn first_layout(files: usize) {
    if enabled() && !LAYOUT_SHOWN.swap(true, Ordering::Relaxed) {
        event(|| format!("first layout shown ({files} files)"));
    }
}

/// One timing line: `[timing] + <ms, right-aligned to 5> ms <what>`.
pub fn line(at: Duration, what: &str) -> String {
    format!("[timing] + {:>5} ms {what}", at.as_millis())
}

/// What a job's status change reads as: `job <name> running`, `job <name> ready (100 ms)`.
pub fn job(name: &str, state: &SourceState) -> String {
    match state {
        SourceState::Running => format!("job {name} running"),
        SourceState::Ready { elapsed } => format!("job {name} ready ({} ms)", elapsed.as_millis()),
        SourceState::Unavailable(_) => format!("job {name} unavailable"),
        SourceState::Failed(_) => format!("job {name} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_read_as_the_gate_expects() {
        assert_eq!(line(Duration::from_millis(812), "first layout shown"), "[timing] +   812 ms first layout shown");
        assert_eq!(line(Duration::from_micros(123_456_789), "x"), "[timing] + 123456 ms x");
        let ready = SourceState::Ready { elapsed: Duration::from_millis(100) };
        assert_eq!(job("git history", &ready), "job git history ready (100 ms)");
        assert_eq!(job("tickets", &SourceState::Running), "job tickets running");
        assert_eq!(job("agent sessions", &SourceState::Unavailable("none".into())), "job agent sessions unavailable");
        assert_eq!(job("git", &SourceState::Failed("x".into())), "job git failed");
    }
}
