//! A plan card: an agent session's plan, its steps (the tasks it created, in order) with the
//! edits made while each was in progress, and the edits made outside any task. Built by the host
//! from the session log at show time; the text lives only in the card.

use std::ops::RangeInclusive;
use std::path::PathBuf;

use studio_ui::CodeDocument;

use super::{LitKind, LitRange};

/// Everything a plan card shows.
#[derive(Debug, Clone)]
pub struct PlanDoc {
    /// The session's id and its generated name.
    pub session: String,
    pub slug: String,
    /// When the session started, as shown ("2026-10-10 14:32").
    pub started: String,
    /// Branches the session worked on.
    pub branches: Vec<String>,
    /// The worktree it worked in, as shown.
    pub worktree: String,
    pub plan: PlanBody,
    /// The session's tasks in the order it created them.
    pub steps: Vec<PlanStep>,
    /// Edits made while no task was in progress, in order.
    pub edits: Vec<PlanEdit>,
    /// Files the session read (only counted).
    pub reads: usize,
}

/// The plan's text, or why there is none.
#[derive(Debug, Clone)]
pub enum PlanBody {
    /// The plan as Markdown; `path` is its file, `note` says where the text came from when it
    /// is not that file (e.g. "plan file deleted (…) · shown from the session log").
    Text { doc: CodeDocument, path: Option<PathBuf>, note: Option<String> },
    /// The plan file is gone and no log line holds the plan.
    Deleted(PathBuf),
    /// The session recorded no plan.
    None,
}

impl PlanBody {
    /// The line shown instead of (or above) the plan text.
    pub fn note(&self) -> Option<String> {
        match self {
            PlanBody::Text { note, .. } => note.clone(),
            PlanBody::Deleted(path) => Some(format!("plan file deleted ({})", path.display())),
            PlanBody::None => Some("no plan recorded".to_string()),
        }
    }
}

/// One step: a task the session created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    /// The task's subject as the log has it (empty when the line no longer holds it).
    pub subject: String,
    /// Its latest state ("pending", "in progress", "completed", ...).
    pub state: String,
    /// The edits made while this task was in progress, in order.
    pub edits: Vec<PlanEdit>,
}

/// One edit the session made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEdit {
    /// The file as the Desk opens it, and as shown (relative to its worktree).
    pub path: PathBuf,
    pub rel: String,
    /// "Edit", "Write" or "Bash edit".
    pub kind: String,
    /// When, as shown ("14:32").
    pub time: String,
    pub matched: EditMatch,
}

/// Where an edit sits in the file as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditMatch {
    /// Found exactly once: these lines (Observed).
    Lit(Vec<RangeInclusive<usize>>),
    /// Found at each of these places; none is lit.
    Candidates(Vec<RangeInclusive<usize>>),
    /// The file no longer holds it; the lines the log gave it, if any.
    Stale(Vec<RangeInclusive<usize>>),
    /// The file is gone.
    Gone,
    /// The log could not say, and why.
    Unavailable(String),
}

impl EditMatch {
    /// What the edit's row says about it.
    pub fn label(&self) -> String {
        match self {
            EditMatch::Lit(ranges) => lines_label(ranges),
            EditMatch::Candidates(ranges) if ranges.len() == 1 => "1 candidate".to_string(),
            EditMatch::Candidates(ranges) => format!("{} candidates", ranges.len()),
            EditMatch::Stale(_) => "stale".to_string(),
            EditMatch::Gone => "file gone".to_string(),
            EditMatch::Unavailable(reason) => format!("unavailable: {reason}"),
        }
    }

    /// Whether the row opens its file: not when the file is gone.
    pub fn opens(&self) -> bool {
        !matches!(self, EditMatch::Gone)
    }
}

/// "lines 3–9", "line 4", or "lines 3–4, 9–9" for several ranges.
fn lines_label(ranges: &[RangeInclusive<usize>]) -> String {
    let one = |r: &RangeInclusive<usize>| {
        if r.start() == r.end() {
            r.start().to_string()
        } else {
            format!("{}–{}", r.start(), r.end())
        }
    };
    match ranges {
        [] => "lit".to_string(),
        [r] if r.start() == r.end() => format!("line {}", r.start()),
        _ => format!("lines {}", ranges.iter().map(one).collect::<Vec<_>>().join(", ")),
    }
}

impl PlanEdit {
    /// What the edit's row lights when clicked: its lines, its candidates, or where it was.
    pub fn lights(&self, slug: &str) -> Vec<LitRange> {
        edit_lights(&self.matched, &format!("session {slug} · {}", self.kind))
    }
}

/// The lights for an edit's match, each saying `source`.
pub fn edit_lights(matched: &EditMatch, source: &str) -> Vec<LitRange> {
    let (ranges, kind, source) = match matched {
        EditMatch::Lit(r) => (r, LitKind::Lit, source.to_string()),
        EditMatch::Candidates(r) => (r, LitKind::Candidate, format!("{source} · one of {} candidates", r.len())),
        EditMatch::Stale(r) => (r, LitKind::Stale, source.to_string()),
        EditMatch::Gone | EditMatch::Unavailable(_) => return Vec::new(),
    };
    ranges.iter().map(|r| LitRange::new(r.clone(), kind, source.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_rows_say_where_the_edit_is() {
        assert_eq!(EditMatch::Lit(vec![3..=9]).label(), "lines 3–9");
        assert_eq!(EditMatch::Lit(vec![4..=4]).label(), "line 4");
        assert_eq!(EditMatch::Lit(vec![3..=4, 9..=9]).label(), "lines 3–4, 9");
        assert_eq!(EditMatch::Candidates(vec![1..=1, 5..=5, 9..=9]).label(), "3 candidates");
        assert_eq!(EditMatch::Stale(vec![]).label(), "stale");
        assert_eq!(EditMatch::Gone.label(), "file gone");
        assert_eq!(EditMatch::Unavailable("log deleted".into()).label(), "unavailable: log deleted");
        assert!(!EditMatch::Gone.opens() && EditMatch::Stale(vec![]).opens());
    }

    #[test]
    fn rows_light_lines_mark_candidates_and_tick_stale_places() {
        let lit = edit_lights(&EditMatch::Lit(vec![3..=4]), "session s · Edit");
        assert_eq!(lit, [LitRange::new(3..=4, LitKind::Lit, "session s · Edit")]);
        let cands = edit_lights(&EditMatch::Candidates(vec![1..=1, 7..=7]), "session s · Edit");
        assert_eq!(cands.len(), 2);
        assert!(cands.iter().all(|l| l.kind == LitKind::Candidate && l.source.ends_with("one of 2 candidates")));
        assert_eq!(edit_lights(&EditMatch::Stale(vec![5..=6]), "x")[0].kind, LitKind::Stale);
        assert!(edit_lights(&EditMatch::Gone, "x").is_empty());
    }
}
