//! The activity line in the middle of the title bar: what Studio is doing right now, with a
//! progress bar while it loads, the last message for a few seconds, and the project's size when
//! nothing is going on.

/// How long a message stays after it was set, in seconds.
pub const MESSAGE_SECONDS: f64 = 4.0;

/// What the activity line shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityLine {
    pub text: String,
    /// 0..1 while something with known progress runs.
    pub progress: Option<f32>,
    /// Something is running (shows a spinner).
    pub busy: bool,
    pub warning: bool,
}

/// Remembers when the status message last changed, so it can fade out.
#[derive(Debug, Default)]
pub struct Activity {
    last_message: Option<String>,
    since: f64,
}

/// What the activity line is worked out from.
pub struct ActivityInputs<'a> {
    pub now: f64,
    /// The loader's stage, files done, files in all and progress, while loading.
    pub loading: Option<(&'a str, usize, usize, f32)>,
    /// Folders loading in the background.
    pub folder_loads: &'a [String],
    pub message: Option<&'a str>,
    pub error: Option<&'a str>,
    /// Files and packages, once a project is open.
    pub project: Option<(usize, usize)>,
}

impl Activity {
    pub fn line(&mut self, inputs: ActivityInputs<'_>) -> ActivityLine {
        if inputs.message != self.last_message.as_deref() {
            self.last_message = inputs.message.map(str::to_string);
            self.since = inputs.now;
        }
        let idle = |text: String| ActivityLine { text, progress: None, busy: false, warning: false };
        if let Some(err) = inputs.error {
            return ActivityLine {
                text: format!("Could not open the project: {err}"),
                warning: true,
                ..idle(String::new())
            };
        }
        if let Some((stage, done, total, progress)) = inputs.loading {
            let text = if total > 0 {
                format!("{stage} · {} of {} files", group(done), group(total))
            } else {
                stage.to_string()
            };
            return ActivityLine { text, progress: Some(progress.clamp(0.0, 1.0)), busy: true, warning: false };
        }
        if let Some(first) = inputs.folder_loads.first() {
            let more = inputs.folder_loads.len() - 1;
            let text =
                if more == 0 { format!("Loading {first}…") } else { format!("Loading {first} and {more} more…") };
            return ActivityLine { text, progress: None, busy: true, warning: false };
        }
        if let Some(message) = &self.last_message {
            if inputs.now - self.since < MESSAGE_SECONDS {
                return idle(message.clone());
            }
        }
        match inputs.project {
            Some((files, packages)) => idle(match packages {
                0 => format!("{} files", group(files)),
                1 => format!("{} files · 1 package", group(files)),
                n => format!("{} files · {n} packages", group(files)),
            }),
            None => idle("No project open".to_string()),
        }
    }
}

/// `16,768`-style counts.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(now: f64) -> ActivityInputs<'static> {
        ActivityInputs {
            now,
            loading: None,
            folder_loads: &[],
            message: None,
            error: None,
            project: Some((16_768, 159)),
        }
    }

    #[test]
    fn loading_shows_progress_then_the_project_size() {
        let mut activity = Activity::default();
        let line = activity.line(ActivityInputs { loading: Some(("Parsing", 4_000, 13_539, 0.4)), ..inputs(0.0) });
        assert_eq!(line.text, "Parsing · 4,000 of 13,539 files");
        assert_eq!(line.progress, Some(0.4));
        assert_eq!(activity.line(inputs(1.0)).text, "16,768 files · 159 packages");
    }

    #[test]
    fn a_message_shows_for_a_few_seconds() {
        let mut activity = Activity::default();
        let message = |now| ActivityInputs { message: Some("Copied: src/main.rs"), ..inputs(now) };
        assert_eq!(activity.line(message(10.0)).text, "Copied: src/main.rs");
        assert_eq!(activity.line(message(13.0)).text, "Copied: src/main.rs");
        assert_eq!(activity.line(message(10.0 + MESSAGE_SECONDS + 0.1)).text, "16,768 files · 159 packages");
    }

    #[test]
    fn errors_and_folder_loads_take_the_line() {
        let mut activity = Activity::default();
        assert!(activity.line(ActivityInputs { error: Some("Path does not exist"), ..inputs(0.0) }).warning);
        let folders = ["node_modules".to_string(), "target".to_string()];
        let line = activity.line(ActivityInputs { folder_loads: &folders, ..inputs(0.0) });
        assert_eq!(line.text, "Loading node_modules and 1 more…");
        assert!(line.busy);
    }
}
