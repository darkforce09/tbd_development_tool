//! Show-time reads: the text the index leaves in the logs (plan text, task subjects, what an
//! edit wrote), read back one log line at a time, and where an edit sits in a file now. Nothing
//! here is cached or logged; callers hold the text only while it is shown, on the Desk's
//! background reader. Bash command text is never read out.

use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use memchr::memmem;
use serde_json::Value;

use super::discover::{Place, Resolver};
use super::parse::parse_time;
use super::{AgentIndex, HunkRange, TaskRef, TaskState, Touch, TouchKind};

const LOG_CHANGED: &str = "log changed";

/// A session's plan text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanText {
    /// The plan file as it is now.
    File { path: PathBuf, text: String },
    /// The plan as the ExitPlanMode line showed it; `deleted` is the plan file, gone since.
    FromLog { text: String, deleted: Option<PathBuf> },
    /// The plan file is gone and no log line shows the plan.
    Deleted(PathBuf),
    /// The session has no plan.
    None,
}

/// One step of a session's plan: a task it created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStep {
    /// Index into the session's tasks.
    pub index: usize,
    /// The TaskCreate subject; empty when the log line no longer holds it.
    pub subject: String,
    /// The latest state TaskUpdate set (pending before the first).
    pub state: TaskState,
}

/// What an edit wrote, as the log has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needle {
    /// An Edit's `new_string`, or a Write's whole `content`.
    Text(String),
    /// A MultiEdit: each edit's `new_string`, in order.
    Texts(Vec<String>),
    /// A Bash-made change: each hunk's line numbers and its added lines.
    Hunks(Vec<(HunkRange, Vec<String>)>),
}

/// Where a touch sits in a file's current text (1-based, inclusive line ranges).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchMatch {
    /// The change, found exactly once (Observed).
    Lit(Vec<RangeInclusive<usize>>),
    /// Found at more than one place: every place, none lit (Possible set n).
    Candidates(Vec<RangeInclusive<usize>>),
    /// The file no longer holds the change.
    Stale,
    /// A read: the lines it asked for (`None`: the whole file).
    ReadOnly(Option<RangeInclusive<usize>>),
    /// The log line could not be read back, and why.
    Unavailable(String),
}

/// The plan text of `session`: the plan file if it still exists, else the ExitPlanMode line's.
pub fn plan_text(index: &AgentIndex, session: usize) -> PlanText {
    let Some(plan) = index.sessions.get(session).and_then(|s| s.plan.as_ref()) else {
        return PlanText::None;
    };
    if let Some(path) = &plan.path {
        if let Ok(text) = std::fs::read_to_string(path) {
            return PlanText::File { path: path.clone(), text };
        }
    }
    if plan.len > 0 {
        let from_log = read_line(index, plan.log, plan.offset, plan.len).ok().filter(|l| same_time(l, plan.time));
        let text = from_log.as_ref().and_then(|line| {
            tool_uses(line).find(|u| u["name"] == "ExitPlanMode").and_then(|u| u["input"]["plan"].as_str())
        });
        if let Some(text) = text {
            return PlanText::FromLog { text: text.to_string(), deleted: plan.path.clone() };
        }
    }
    match &plan.path {
        Some(path) => PlanText::Deleted(path.clone()),
        None => PlanText::None,
    }
}

/// The tasks `session` created, in TaskCreate order, with their subjects and latest states.
pub fn task_steps(index: &AgentIndex, session: usize) -> Vec<TaskStep> {
    let Some(s) = index.sessions.get(session) else { return Vec::new() };
    s.tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            // Several TaskCreate calls may share one line: this task is the nth of them.
            let nth = s.tasks[..i].iter().filter(|t| (t.log, t.offset) == (task.log, task.offset)).count();
            TaskStep {
                index: i,
                subject: task_subject(index, task, nth).unwrap_or_default(),
                state: task.states.last().map_or(TaskState::Pending, |&(_, state)| state),
            }
        })
        .collect()
}

fn task_subject(index: &AgentIndex, task: &TaskRef, nth: usize) -> Option<String> {
    let line = read_line(index, task.log, task.offset, task.len).ok().filter(|l| same_time(l, task.created))?;
    let create = tool_uses(&line).filter(|u| u["name"] == "TaskCreate").nth(nth)?;
    create["input"]["subject"].as_str().map(str::to_string)
}

/// What touch `touch` wrote (`None` for reads, or when its line cannot be read back).
pub fn touch_needle(index: &AgentIndex, touch: usize) -> Option<Needle> {
    let t = index.touches.get(touch)?;
    match t.kind {
        TouchKind::Read => None,
        TouchKind::Edit | TouchKind::Write => {
            let call = tool_call(index, t, None).ok()?;
            match edits(&call)? {
                Edits::Single(text, _) => Some(Needle::Text(text)),
                Edits::Multi(edits) => Some(Needle::Texts(edits.into_iter().map(|(text, _)| text).collect())),
            }
        }
        TouchKind::BashEdit => {
            let (hunks, _) = bash_hunks(index, t).ok()?;
            Some(Needle::Hunks(hunks.into_iter().map(|h| (h.range, h.added().map(str::to_string).collect())).collect()))
        }
    }
}

/// Where touch `touch` sits in `file_text` (the touched file as it is now), by the exact-match
/// rule: the change found once is lit; found n times, n candidates; not found, stale.
pub fn match_touch(index: &AgentIndex, touch: usize, file_text: &str) -> TouchMatch {
    let Some(t) = index.touches.get(touch) else {
        return TouchMatch::Unavailable("no such touch".to_string());
    };
    let file = FileText::new(file_text);
    if t.kind == TouchKind::Read {
        return TouchMatch::ReadOnly(read_range(t.read, file.line_count()));
    }
    let result = match t.kind {
        TouchKind::BashEdit => bash_hunks(index, t).and_then(|(hunks, deleted)| match (hunks.is_empty(), deleted) {
            // The call deleted the file: a file there now is not what it left.
            (_, true) => Ok(TouchMatch::Stale),
            (true, false) => Err("no hunks in the log".to_string()),
            (false, false) => Ok(match_hunks(&file, &hunks)),
        }),
        _ => match_edit(index, t, &file),
    };
    result.unwrap_or_else(TouchMatch::Unavailable)
}

fn match_edit(index: &AgentIndex, t: &Touch, file: &FileText) -> Result<TouchMatch, String> {
    let call = tool_call(index, t, None)?;
    let edits = edits(&call).ok_or(LOG_CHANGED)?;
    if let Edits::Single(text, true) = &edits {
        // Every occurrence is a real edit.
        let found = file.find_text(text);
        return Ok(if found.is_empty() { TouchMatch::Stale } else { TouchMatch::Lit(found) });
    }
    if !t.hunks.is_empty() {
        let id = call["id"].as_str().unwrap_or_default();
        if let Some(hunks) = result_patch(index, t, id).filter(|h| h.iter().any(|h| !h.new_side.is_empty())) {
            return Ok(match_hunks(file, &hunks));
        }
    }
    let pieces = match edits {
        Edits::Single(text, _) => vec![text_piece(file, &text, &t.hunks)],
        Edits::Multi(edits) => edits
            .iter()
            .map(|(text, all)| {
                if !*all {
                    return text_piece(file, text, &[]);
                }
                match file.find_text(text) {
                    found if found.is_empty() => Piece::Zero,
                    found => Piece::Once(found),
                }
            })
            .collect(),
    };
    Ok(combine(pieces))
}

/// The exact-match rule for one text; an empty text stands at its hunks' places, if any.
fn text_piece(file: &FileText, text: &str, hunks: &[HunkRange]) -> Piece {
    if text.is_empty() {
        if hunks.is_empty() {
            return Piece::Zero;
        }
        return Piece::Once(hunks.iter().map(|h| hunk_span(h.new_start, h.new_lines)).collect());
    }
    Piece::from_found(file.find_text(text))
}

// ---------------------------------------------------------------------------------------------
// Log lines.

/// Reads exactly the bytes of one line and parses it.
fn read_line(index: &AgentIndex, log: usize, offset: u64, len: u32) -> Result<Value, String> {
    let path = &index.logs.get(log).ok_or(LOG_CHANGED)?.path;
    let mut file = File::open(path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => "log deleted".to_string(),
        _ => format!("log unreadable: {e}"),
    })?;
    file.seek(SeekFrom::Start(offset)).map_err(|_| LOG_CHANGED)?;
    let mut buf = vec![0u8; len as usize];
    file.read_exact(&mut buf).map_err(|_| LOG_CHANGED)?;
    serde_json::from_slice(&buf).map_err(|_| LOG_CHANGED.to_string())
}

/// The line still has the time the index saw (0: the index saw none).
fn same_time(line: &Value, expected: i64) -> bool {
    let time = line["timestamp"].as_str().and_then(|t| parse_time(t.as_bytes()));
    expected == 0 || time == Some(expected)
}

fn content(line: &Value) -> impl Iterator<Item = &Value> {
    line.pointer("/message/content").and_then(Value::as_array).into_iter().flatten()
}

/// The tool calls of an assistant line.
fn tool_uses(line: &Value) -> impl Iterator<Item = &Value> {
    let assistant = line["type"] == "assistant";
    content(line).filter(move |i| assistant && i["type"] == "tool_use")
}

/// The touch's tool call, checked against the index: an assistant line at the same time with a
/// call of the touch's kind on the touch's file (for a Bash edit: the call `answered` names).
fn tool_call(index: &AgentIndex, t: &Touch, answered: Option<&[&str]>) -> Result<Value, String> {
    let line = read_line(index, t.log, t.offset, t.len)?;
    if !same_time(&line, t.time) {
        return Err(LOG_CHANGED.to_string());
    }
    let names: &[&str] = match t.kind {
        TouchKind::Read => &["Read"],
        TouchKind::Edit => &["Edit", "MultiEdit"],
        TouchKind::Write => &["Write"],
        TouchKind::BashEdit => &["Bash"],
    };
    let resolver = Resolver::new(index.roots.clone(), false);
    let cwd = line["cwd"].as_str();
    let call = tool_uses(&line).find(|u| {
        let named = u["name"].as_str().is_some_and(|n| names.contains(&n));
        let target = match answered {
            Some(ids) => u["id"].as_str().is_some_and(|id| ids.contains(&id)),
            None => u["input"]["file_path"].as_str().is_some_and(|p| names_file(&resolver, p, cwd, &t.path)),
        };
        named && target
    });
    call.cloned().ok_or_else(|| LOG_CHANGED.to_string())
}

/// `path` (relative ones against `cwd`) is `file`.
fn names_file(resolver: &Resolver, path: &str, cwd: Option<&str>, file: &Path) -> bool {
    match resolver.locate(path, cwd) {
        Place::Inside(p) | Place::Outside(p) => Path::new(&p) == file,
        Place::Unknown => false,
    }
}

/// The touch's result line, checked to answer the call `id` (any call when `None`).
fn result_line(index: &AgentIndex, t: &Touch, id: Option<&str>) -> Result<Value, String> {
    let (log, offset, len) = t.result.ok_or("no result in the log")?;
    let line = read_line(index, log, offset, len)?;
    let answers = line["type"] == "user"
        && content(&line).any(|i| i["type"] == "tool_result" && id.is_none_or(|id| i["tool_use_id"] == id));
    if answers {
        Ok(line)
    } else {
        Err(LOG_CHANGED.to_string())
    }
}

enum Edits {
    /// `new_string` (or `content`) and `replace_all`.
    Single(String, bool),
    Multi(Vec<(String, bool)>),
}

fn edits(call: &Value) -> Option<Edits> {
    let input = &call["input"];
    let all = |v: &Value| v["replace_all"].as_bool().unwrap_or(false);
    match call["name"].as_str()? {
        "Edit" => Some(Edits::Single(input["new_string"].as_str()?.to_string(), all(input))),
        "Write" => Some(Edits::Single(input["content"].as_str()?.to_string(), false)),
        "MultiEdit" => {
            let list = input["edits"].as_array()?;
            list.iter()
                .map(|e| Some((e["new_string"].as_str()?.to_string(), all(e))))
                .collect::<Option<_>>()
                .map(Edits::Multi)
        }
        _ => None,
    }
}

/// One hunk with its new side: each line, and whether the change added it.
#[derive(Debug, Clone)]
struct Hunk {
    range: HunkRange,
    new_side: Vec<(bool, String)>,
}

impl Hunk {
    fn parse(v: &Value) -> Hunk {
        let num = |k: &str| match &v[k] {
            Value::Number(n) => n.as_u64().unwrap_or(0).min(u32::MAX as u64) as u32,
            Value::String(s) => s.trim().parse().unwrap_or(0),
            _ => 0,
        };
        let range = HunkRange {
            old_start: num("oldStart"),
            old_lines: num("oldLines"),
            new_start: num("newStart"),
            new_lines: num("newLines"),
        };
        let lines = v["lines"].as_array().map(Vec::as_slice).unwrap_or_default();
        let new_side = lines
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|l| match l.as_bytes().first() {
                Some(b'+') => Some((true, l[1..].to_string())),
                Some(b' ') => Some((false, l[1..].to_string())),
                Some(b'-' | b'\\') => None,
                _ => Some((false, l.to_string())),
            })
            .collect();
        Hunk { range, new_side }
    }

    fn added(&self) -> impl Iterator<Item = &str> {
        self.new_side.iter().filter(|(added, _)| *added).map(|(_, l)| l.as_str())
    }
}

/// The `structuredPatch` of an Edit or Write's result line.
fn result_patch(index: &AgentIndex, t: &Touch, id: &str) -> Option<Vec<Hunk>> {
    let line = result_line(index, t, Some(id)).ok()?;
    let patch = line["toolUseResult"]["structuredPatch"].as_array()?;
    Some(patch.iter().map(Hunk::parse).collect())
}

/// The hunks a Bash call made to the touch's file (`bashEditDiff`), and whether it deleted the
/// file; the command is not read.
fn bash_hunks(index: &AgentIndex, t: &Touch) -> Result<(Vec<Hunk>, bool), String> {
    let result = result_line(index, t, None)?;
    let ids: Vec<&str> =
        content(&result).filter(|i| i["type"] == "tool_result").filter_map(|i| i["tool_use_id"].as_str()).collect();
    // The call must be a Bash call this result answers.
    tool_call(index, t, Some(&ids))?;
    let resolver = Resolver::new(index.roots.clone(), false);
    let cwd = result["cwd"].as_str();
    let files = result["toolUseResult"]["bashEditDiff"]["files"].as_array().ok_or(LOG_CHANGED)?;
    let mine = files.iter().filter(|f| f["filePath"].as_str().is_some_and(|p| names_file(&resolver, p, cwd, &t.path)));
    let hunks_of = |f: &Value| -> (Vec<Hunk>, bool) {
        let hunks = f["hunks"].as_array().map(|h| h.iter().map(Hunk::parse).collect()).unwrap_or_default();
        (hunks, f["deleted"].as_bool().unwrap_or(false))
    };
    let all: Vec<(Vec<Hunk>, bool)> = mine.map(hunks_of).collect();
    let ranges = |hunks: &[Hunk]| hunks.iter().map(|h| h.range).collect::<Vec<_>>();
    all.iter().find(|(h, _)| ranges(h) == t.hunks).or(all.first()).cloned().ok_or_else(|| LOG_CHANGED.to_string())
}

// ---------------------------------------------------------------------------------------------
// Matching.

/// A file's text with `\r\n` read as `\n`, and where each line starts.
struct FileText {
    text: String,
    starts: Vec<usize>,
}

impl FileText {
    fn new(text: &str) -> Self {
        let text = text.replace("\r\n", "\n");
        let mut starts = vec![0];
        starts.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1).filter(|&i| i < text.len()));
        Self { text, starts }
    }

    fn line_count(&self) -> usize {
        if self.text.is_empty() {
            0
        } else {
            self.starts.len()
        }
    }

    fn line(&self, i: usize) -> &str {
        let start = self.starts[i];
        let end = self.starts.get(i + 1).map_or(self.text.len(), |&e| e);
        self.text[start..end].strip_suffix('\n').unwrap_or(&self.text[start..end])
    }

    /// 0-based line of byte `at`.
    fn line_of(&self, at: usize) -> usize {
        self.starts.partition_point(|&s| s <= at) - 1
    }

    /// Every place `needle` occurs (overlapping too), as 1-based line ranges.
    fn find_text(&self, needle: &str) -> Vec<RangeInclusive<usize>> {
        let needle = needle.replace("\r\n", "\n");
        if needle.is_empty() {
            return Vec::new();
        }
        let finder = memmem::Finder::new(needle.as_bytes());
        let hay = self.text.as_bytes();
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(at) = finder.find(&hay[from..]).map(|i| from + i) {
            found.push(self.line_of(at) + 1..=self.line_of(at + needle.len() - 1) + 1);
            from = at + 1;
        }
        found
    }

    /// Whether `lines` sit at 0-based line `at`.
    fn lines_at(&self, at: usize, lines: &[&str]) -> bool {
        at + lines.len() <= self.line_count() && lines.iter().enumerate().all(|(k, l)| self.line(at + k) == *l)
    }

    /// Every place the block of whole `lines` occurs, as 1-based line ranges.
    fn find_lines(&self, lines: &[&str]) -> Vec<RangeInclusive<usize>> {
        if lines.is_empty() || lines.len() > self.line_count() {
            return Vec::new();
        }
        (0..=self.line_count() - lines.len())
            .filter(|&at| self.lines_at(at, lines))
            .map(|at| at + 1..=at + lines.len())
            .collect()
    }
}

/// One part of a change: found once, at several places, or not at all.
enum Piece {
    Once(Vec<RangeInclusive<usize>>),
    Many(Vec<RangeInclusive<usize>>),
    Zero,
}

impl Piece {
    fn from_found(found: Vec<RangeInclusive<usize>>) -> Piece {
        match found.len() {
            0 => Piece::Zero,
            1 => Piece::Once(found),
            _ => Piece::Many(found),
        }
    }
}

/// Lit when every part is found once; stale when any part is gone; else every candidate.
fn combine(pieces: Vec<Piece>) -> TouchMatch {
    if pieces.iter().any(|p| matches!(p, Piece::Zero)) {
        return TouchMatch::Stale;
    }
    let many = pieces.iter().any(|p| matches!(p, Piece::Many(_)));
    let ranges = pieces.into_iter().flat_map(|p| match p {
        Piece::Once(r) | Piece::Many(r) => r,
        Piece::Zero => Vec::new(),
    });
    if many {
        TouchMatch::Candidates(ranges.collect())
    } else {
        TouchMatch::Lit(ranges.collect())
    }
}

/// Each hunk where its line numbers say, if its new side is still there; else its added block,
/// searched for.
fn match_hunks(file: &FileText, hunks: &[Hunk]) -> TouchMatch {
    combine(hunks.iter().map(|h| hunk_piece(file, h)).collect())
}

fn hunk_piece(file: &FileText, h: &Hunk) -> Piece {
    let start = h.range.new_start.max(1) as usize - 1;
    let new_side: Vec<&str> = h.new_side.iter().map(|(_, l)| l.as_str()).collect();
    let first = h.new_side.iter().position(|(added, _)| *added);
    let last = h.new_side.iter().rposition(|(added, _)| *added);
    if new_side.is_empty() {
        return Piece::Once(vec![hunk_span(h.range.new_start, 0)]);
    }
    if file.lines_at(start, &new_side) {
        return Piece::Once(vec![match (first, last) {
            (Some(a), Some(b)) => start + a + 1..=start + b + 1,
            _ => start + 1..=start + new_side.len(),
        }]);
    }
    match (first, last) {
        (Some(a), Some(b)) => Piece::from_found(file.find_lines(&new_side[a..=b])),
        _ => Piece::Zero,
    }
}

/// A hunk's new-side lines; a hunk with none stands at its start line.
fn hunk_span(new_start: u32, new_lines: u32) -> RangeInclusive<usize> {
    let start = new_start.max(1) as usize;
    start..=start + (new_lines.max(1) as usize) - 1
}

/// The lines a Read asked for: `offset` is the first line, `limit` how many (0: to the end).
fn read_range(read: Option<(u32, u32)>, line_count: usize) -> Option<RangeInclusive<usize>> {
    let (offset, limit) = read?;
    if offset == 0 && limit == 0 {
        return None;
    }
    let start = offset.max(1) as usize;
    let end = if limit == 0 { line_count.max(start) } else { start + limit as usize - 1 };
    Some(start..=end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(new_start: u32, lines: &[&str]) -> Hunk {
        let v = serde_json::json!({ "oldStart": new_start, "oldLines": 1, "newStart": new_start,
            "newLines": lines.len(), "lines": lines });
        Hunk::parse(&v)
    }

    #[test]
    fn text_matches_across_lines_and_crlf() {
        let file = FileText::new("a\r\nfn x() {\r\n    1\r\n}\r\nb\r\n");
        assert_eq!(file.line_count(), 5);
        assert_eq!(file.find_text("fn x() {\n    1\n}"), vec![2..=4]);
        assert_eq!(file.find_text("fn x() {\r\n    1\r\n}\r\n"), vec![2..=4]);
        assert_eq!(file.find_text("1\n}\nb\n"), vec![3..=5]);
        assert!(file.find_text("missing").is_empty());
        // Overlapping occurrences count.
        assert_eq!(FileText::new("aaa").find_text("aa").len(), 2);
    }

    #[test]
    fn hunks_check_their_place_then_search() {
        let h = [hunk(2, &[" ctx", "-old", "+new one", "+new two", " after"])];
        let file = FileText::new("top\nctx\nnew one\nnew two\nafter\n");
        assert_eq!(match_hunks(&file, &h), TouchMatch::Lit(vec![3..=4]));
        let moved = FileText::new("x\ny\nz\nctx\nnew one\nnew two\n");
        assert_eq!(match_hunks(&moved, &h), TouchMatch::Lit(vec![5..=6]));
        let twice = FileText::new("new one\nnew two\n\nnew one\nnew two\n");
        assert_eq!(match_hunks(&twice, &h), TouchMatch::Candidates(vec![1..=2, 4..=5]));
        assert_eq!(match_hunks(&FileText::new("other\n"), &h), TouchMatch::Stale);
        // A deletion: its context stands where it was.
        let del = [hunk(1, &[" keep", "-gone", " also"])];
        assert_eq!(match_hunks(&FileText::new("keep\nalso\n"), &del), TouchMatch::Lit(vec![1..=2]));
        assert_eq!(match_hunks(&FileText::new("changed\nalso\n"), &del), TouchMatch::Stale);
    }

    /// A one-session index over `lines` (call, result pairs) in a temp log, all on `file`.
    fn synthetic(dir: &Path, lines: &[String]) -> AgentIndex {
        use super::super::{LogFile, Touch};
        let log = dir.join("log.jsonl");
        let mut text = String::new();
        let mut at = Vec::new();
        for l in lines {
            at.push((text.len() as u64, l.len() as u32));
            text.push_str(l);
            text.push('\n');
        }
        std::fs::write(&log, text).unwrap();
        let touches = (0..lines.len() / 2)
            .map(|i| Touch {
                session: 0,
                path: dir.join("f.rs"),
                kind: TouchKind::Edit,
                time: 0,
                log: 0,
                offset: at[2 * i].0,
                len: at[2 * i].1,
                result: Some((0, at[2 * i + 1].0, at[2 * i + 1].1)),
                read: None,
                hunks: vec![HunkRange { old_start: 2, old_lines: 3, new_start: 2, new_lines: 2 }],
                subagent: false,
                task: None,
            })
            .collect();
        AgentIndex {
            roots: vec![dir.to_path_buf()],
            logs: vec![LogFile { path: log, subagent: false, session: Some(0) }],
            touches,
            ..Default::default()
        }
    }

    #[test]
    fn deletions_and_multi_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).unwrap();
        let file = dir.join("f.rs");
        let call = |id: &str, name: &str, input: Value| {
            serde_json::json!({"type": "assistant", "message": {"content": [
                {"type": "tool_use", "id": id, "name": name, "input": input}]}})
            .to_string()
        };
        let result = |id: &str, patch: Value| {
            serde_json::json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": id}]},
                "toolUseResult": {"structuredPatch": patch}})
            .to_string()
        };
        let index = synthetic(
            &dir,
            &[
                call("d", "Edit", serde_json::json!({"file_path": file, "old_string": "x", "new_string": ""})),
                result(
                    "d",
                    serde_json::json!([{"oldStart": 2, "oldLines": 3, "newStart": 2, "newLines": 2,
                    "lines": [" a", "-x", " b"]}]),
                ),
                call(
                    "m",
                    "MultiEdit",
                    serde_json::json!({"file_path": file, "edits": [
                    {"old_string": "1", "new_string": "one\ntwo"}, {"old_string": "2", "new_string": "end"}]}),
                ),
                result("m", serde_json::json!([])),
            ],
        );
        // An empty new_string stands where its hunk is, while its context is still there.
        assert_eq!(match_touch(&index, 0, "top\na\nb\n"), TouchMatch::Lit(vec![2..=3]));
        assert_eq!(match_touch(&index, 0, "top\nz\nb\n"), TouchMatch::Stale);
        assert_eq!(touch_needle(&index, 1), Some(Needle::Texts(vec!["one\ntwo".into(), "end".into()])));
        assert_eq!(match_touch(&index, 1, "one\r\ntwo\r\nend\r\n"), TouchMatch::Lit(vec![1..=2, 3..=3]));
        assert_eq!(match_touch(&index, 1, "one\ntwo\nend\nend\n"), TouchMatch::Candidates(vec![1..=2, 3..=3, 4..=4]));
        assert_eq!(match_touch(&index, 1, "one\ntwo\n"), TouchMatch::Stale);
        // A result line answering another call is not read: the hunk position stands.
        let index = synthetic(
            &dir,
            &[
                call("d", "Edit", serde_json::json!({"file_path": file, "new_string": ""})),
                result("other", serde_json::json!([])),
            ],
        );
        assert_eq!(match_touch(&index, 0, "a\nb\n"), TouchMatch::Lit(vec![2..=3]));
    }

    #[test]
    fn read_ranges() {
        assert_eq!(read_range(None, 9), None);
        assert_eq!(read_range(Some((0, 0)), 9), None);
        assert_eq!(read_range(Some((10, 20)), 9), Some(10..=29));
        assert_eq!(read_range(Some((0, 5)), 9), Some(1..=5));
        assert_eq!(read_range(Some((4, 0)), 9), Some(4..=9));
    }
}
