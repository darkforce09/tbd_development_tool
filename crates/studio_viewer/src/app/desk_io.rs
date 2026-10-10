//! Reads what the Desk asks for, off the UI thread: files as card contents, agent sessions' plan
//! cards (plan text, task subjects and edits read from the session log at show time, kept only
//! in the card), and where the chosen session's edits sit in the files open on the Desk.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use studio_canvas::desk::plan::edit_lights;
use studio_canvas::desk::{EditMatch, LitRange, PlanBody, PlanDoc, PlanEdit, PlanStep};
use studio_canvas::{content_for_text, CameraTarget, CardContent, DeskRequest, Stop};
use studio_sources::agents::{match_touch, plan_text, task_steps, PlanText, TouchMatch};
use studio_sources::{AgentIndex, TouchKind};
use studio_ui::CodeDocument;

use super::StudioApp;

/// Files above this are not shown as text.
pub const MAX_DESK_BYTES: u64 = 4 * 1024 * 1024;

/// What comes back from the reader threads.
enum DeskReply {
    /// A card's contents.
    Content(u64, CardContent),
    /// The lights of session `session` (id) on a card.
    SessionLights { card: u64, session: String, lights: Vec<LitRange> },
}

/// The session whose edits are lit on the open cards.
struct LitSession {
    id: String,
    index: Arc<AgentIndex>,
    /// The session's edits (touch indexes) by file.
    files: HashMap<PathBuf, Vec<usize>>,
    /// Cards whose lights were asked for.
    cards: HashSet<u64>,
}

/// The channel finished reads come back on, and the lit session.
pub struct DeskReader {
    tx: Sender<DeskReply>,
    rx: Receiver<DeskReply>,
    lit: Option<LitSession>,
    /// The (session id, index) the lights were last set for.
    lit_for: Option<(String, usize)>,
}

impl Default for DeskReader {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, lit: None, lit_for: None }
    }
}

/// Reads a file the way the Desk shows it: text, Markdown, binary or too large.
pub fn read_for_desk(path: &Path) -> CardContent {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) => return CardContent::Unreadable(e.to_string()),
    };
    if size > MAX_DESK_BYTES {
        return CardContent::TooLarge(size);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    if let Err(e) = std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)) {
        return CardContent::Unreadable(e.to_string());
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return CardContent::Binary(size);
    }
    let text: Arc<str> = String::from_utf8_lossy(&bytes).into();
    content_for_text(path, text)
}

/// How an edit's kind is shown.
fn kind_label(kind: TouchKind) -> &'static str {
    match kind {
        TouchKind::Read => "Read",
        TouchKind::Edit => "Edit",
        TouchKind::Write => "Write",
        TouchKind::BashEdit => "Bash edit",
    }
}

/// Unix ms as local "YYYY-MM-DD HH:MM" (`offset_secs` ahead of UTC).
pub fn local_date_time(ms: i64, offset_secs: i64) -> String {
    let secs = ms.div_euclid(1000) + offset_secs;
    let (days, day_secs) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", day_secs / 3600, day_secs % 3600 / 60)
}

/// The text of a file an edit touched, read once per plan: `Err` says why there is none.
fn file_text(path: &Path) -> Result<String, EditMatch> {
    // Most old touches point at files that are gone: check before reading.
    let meta = std::fs::metadata(path).map_err(|_| EditMatch::Gone)?;
    if !meta.is_file() {
        return Err(EditMatch::Gone);
    }
    if meta.len() > MAX_DESK_BYTES {
        return Err(EditMatch::Unavailable("file too large".to_string()));
    }
    std::fs::read_to_string(path).map_err(|_| EditMatch::Unavailable("not text".to_string()))
}

/// Where touch `touch` sits in `text`: the exact-match rule, with a stale edit placed where the
/// log's line numbers put it (if it gave any), kept within the file so its tick shows.
fn edit_match(index: &AgentIndex, touch: usize, text: &str) -> EditMatch {
    match match_touch(index, touch, text) {
        TouchMatch::Lit(ranges) => EditMatch::Lit(ranges),
        TouchMatch::Candidates(ranges) => EditMatch::Candidates(ranges),
        TouchMatch::Stale => {
            let last = text.lines().count().max(1);
            let hunks = index.touches.get(touch).map_or(&[][..], |t| &t.hunks[..]);
            let ranges = hunks
                .iter()
                .filter(|h| h.new_start > 0)
                .map(|h| {
                    let start = (h.new_start as usize).min(last);
                    start..=(start + h.new_lines.max(1) as usize - 1).min(last)
                })
                .collect();
            EditMatch::Stale(ranges)
        }
        TouchMatch::ReadOnly(_) => EditMatch::Unavailable("a read".to_string()),
        TouchMatch::Unavailable(reason) => EditMatch::Unavailable(reason),
    }
}

/// `path` shown relative to the worktree it is in (the longest root holding it).
fn rel_to_roots(roots: &[PathBuf], path: &Path) -> String {
    roots
        .iter()
        .filter_map(|r| path.strip_prefix(r).ok())
        .min_by_key(|rel| rel.as_os_str().len())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The session's display name: its slug, else the start of its id.
fn session_name(index: &AgentIndex, session: usize) -> String {
    let s = &index.sessions[session];
    s.slug.clone().unwrap_or_else(|| s.id.chars().take(8).collect())
}

/// Builds the plan card of `session`: its plan, its tasks in order each with the edits made
/// while it was in progress, the edits made outside any task (in order), and how many reads.
/// Reads the plan, the task lines, the edits' log lines and the files they touched.
pub fn build_plan_doc(index: &AgentIndex, session: usize, offset_secs: i64) -> PlanDoc {
    let s = &index.sessions[session];
    let plan = match plan_text(index, session) {
        PlanText::File { path, text } => {
            PlanBody::Text { doc: CodeDocument::new(text, "md"), path: Some(path), note: None }
        }
        PlanText::FromLog { text, deleted } => {
            let note = match &deleted {
                Some(path) => format!("plan file deleted ({}) · shown from the session log", path.display()),
                None => "plan shown from the session log".to_string(),
            };
            PlanBody::Text { doc: CodeDocument::new(text, "md"), path: deleted, note: Some(note) }
        }
        PlanText::Deleted(path) => PlanBody::Deleted(path),
        PlanText::None => PlanBody::None,
    };
    let task_steps = task_steps(index, session);
    let mut steps: Vec<PlanStep> = task_steps
        .iter()
        .map(|t| PlanStep { subject: t.subject.clone(), state: t.state.label().to_string(), edits: Vec::new() })
        .collect();
    let step_of: HashMap<usize, usize> = task_steps.iter().enumerate().map(|(i, t)| (t.index, i)).collect();

    let mut touches: Vec<usize> = (0..index.touches.len()).filter(|&i| index.touches[i].session == session).collect();
    touches.sort_by_key(|&i| (index.touches[i].time, i));
    let mut texts: HashMap<&Path, Result<String, EditMatch>> = HashMap::new();
    let mut edits = Vec::new();
    let mut reads = 0;
    for i in touches {
        let t = &index.touches[i];
        if t.kind == TouchKind::Read {
            reads += 1;
            continue;
        }
        let matched = match texts.entry(&t.path).or_insert_with(|| file_text(&t.path)) {
            Ok(text) => edit_match(index, i, text),
            Err(why) => why.clone(),
        };
        let edit = PlanEdit {
            path: t.path.clone(),
            rel: rel_to_roots(&index.roots, &t.path),
            kind: kind_label(t.kind).to_string(),
            time: local_date_time(t.time, offset_secs)[11..].to_string(),
            matched,
        };
        match t.task.and_then(|task| step_of.get(&task)) {
            Some(&step) => steps[step].edits.push(edit),
            None => edits.push(edit),
        }
    }
    PlanDoc {
        session: s.id.clone(),
        slug: session_name(index, session),
        started: local_date_time(s.started, offset_secs),
        branches: s.branches.clone(),
        worktree: index.roots.get(s.root).map_or_else(String::new, |r| r.display().to_string()),
        plan,
        steps,
        edits,
        reads,
    }
}

/// The session's edits (touch indexes) by file.
fn edits_by_file(index: &AgentIndex, session: usize) -> HashMap<PathBuf, Vec<usize>> {
    let mut files: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (i, t) in index.touches.iter().enumerate() {
        if t.session == session && t.kind != TouchKind::Read {
            files.entry(t.path.clone()).or_default().push(i);
        }
    }
    files
}

/// The lights of `touches` (one session's edits of one file) on the file's `text`.
pub fn card_lights(index: &AgentIndex, touches: &[usize], text: &str) -> Vec<LitRange> {
    let mut lights = Vec::new();
    for &i in touches {
        let Some(t) = index.touches.get(i) else { continue };
        let source = format!("session {} · {}", session_name(index, t.session), kind_label(t.kind));
        lights.extend(edit_lights(&edit_match(index, i, text), &source));
    }
    lights
}

impl StudioApp {
    /// Starts reading what the Desk asked for, keeps the chosen session's lights on the open
    /// cards and the map, and fills in what has been read. Called every frame; work starts only
    /// when a card opens or the session changes.
    pub(crate) fn poll_desk_reads(&mut self, ctx: &eframe::egui::Context) {
        let agents = self.sources.as_ref().and_then(|s| s.agents.clone());
        for request in std::mem::take(&mut self.canvas_state.desk.requests) {
            let tx = self.desk_reader.tx.clone();
            let ctx = ctx.clone();
            match request {
                DeskRequest::Read { card, path } => {
                    std::thread::spawn(move || {
                        let _ = tx.send(DeskReply::Content(card, read_for_desk(&path)));
                        ctx.request_repaint();
                    });
                }
                DeskRequest::Plan { card, session } => {
                    let found = agents.clone().and_then(|a| {
                        let s = a.sessions.iter().position(|s| s.id == session)?;
                        Some((a, s))
                    });
                    let Some((index, s)) = found else {
                        let gone = "This session is not in the agents index".to_string();
                        self.canvas_state.desk.fill(card, CardContent::Unreadable(gone));
                        continue;
                    };
                    let offset = super::title_bar::local_offset_secs(super::sources::now_unix_ms() / 1000);
                    std::thread::spawn(move || {
                        let doc = build_plan_doc(&index, s, offset);
                        let _ = tx.send(DeskReply::Content(card, CardContent::Plan(Box::new(doc))));
                        ctx.request_repaint();
                    });
                }
            }
        }
        self.sync_session_lights(agents, ctx);
        while let Ok(reply) = self.desk_reader.rx.try_recv() {
            match reply {
                DeskReply::Content(card, content) => self.canvas_state.desk.fill(card, content),
                DeskReply::SessionLights { card, session, lights } => {
                    if self.desk_reader.lit.as_ref().is_some_and(|l| l.id == session) {
                        self.canvas_state.desk.set_session_lights(card, lights);
                    }
                }
            }
        }
    }

    /// Follows the session chosen in the Changes district: on a change of session (or of the
    /// agents index) the old lights go and the new session's files are set for the map; each
    /// open card of a file it edited gets its lights worked out once, on a reader thread.
    fn sync_session_lights(&mut self, agents: Option<Arc<AgentIndex>>, ctx: &eframe::egui::Context) {
        let wanted = self.canvas_state.session_lit.clone();
        let key = wanted.clone().zip(agents.as_ref().map(|a| Arc::as_ptr(a) as usize));
        let reader = &mut self.desk_reader;
        if key != reader.lit_for {
            reader.lit_for = key;
            reader.lit = wanted.as_ref().zip(agents).and_then(|(id, index)| {
                let session = index.sessions.iter().position(|s| &s.id == id)?;
                let files = edits_by_file(&index, session);
                Some(LitSession { id: id.clone(), index, files, cards: HashSet::new() })
            });
            let desk = &mut self.canvas_state.desk;
            desk.clear_session_lights();
            let paths = reader.lit.as_ref().map_or_else(Vec::new, |l| {
                let mut paths: Vec<String> = l.files.keys().map(|p| p.to_string_lossy().into_owned()).collect();
                paths.sort();
                paths
            });
            desk.session.set(reader.lit.as_ref().map(|l| l.id.clone()), paths);
        }
        if let Some(lit) = &mut reader.lit {
            for card in &self.canvas_state.desk.cards {
                let Some(doc) = card.document() else { continue };
                let Some(touches) = lit.files.get(&card.path) else { continue };
                if !lit.cards.insert(card.id) {
                    continue;
                }
                let (tx, ctx, index) = (reader.tx.clone(), ctx.clone(), lit.index.clone());
                let (card, session, touches, text) = (card.id, lit.id.clone(), touches.clone(), doc.text.clone());
                std::thread::spawn(move || {
                    let lights = card_lights(&index, &touches, &text);
                    let _ = tx.send(DeskReply::SessionLights { card, session, lights });
                    ctx.request_repaint();
                });
            }
        }
        let revision = self.canvas_state.scene.revision;
        if let Some(nodes) = self.canvas_state.desk.session.refresh(&self.graph, revision) {
            self.canvas_state.session_lit_nodes = nodes;
        }
    }

    /// Opens an agent session's plan card on the Desk and goes there.
    pub(crate) fn open_session_plan(&mut self, id: String) {
        let index = self.sources.as_ref().and_then(|s| s.agents.as_deref());
        let name = index
            .and_then(|a| a.sessions.iter().position(|s| s.id == id).map(|s| session_name(a, s)))
            .unwrap_or_else(|| id.chars().take(8).collect());
        self.canvas_state.desk.open_plan(&id, &name);
        self.canvas_state.fly_to(CameraTarget::Stop(Stop::Desk));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_canvas::desk::LitKind;

    #[test]
    fn files_are_read_as_what_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        assert!(matches!(read_for_desk(&write("a.rs", b"fn a() {}\n")), CardContent::Code(d) if d.line_count() == 1));
        assert!(matches!(read_for_desk(&write("README.md", b"# Hi\n")), CardContent::Markdown(_)));
        assert!(matches!(read_for_desk(&write("blob.bin", &[1, 0, 2])), CardContent::Binary(3)));
        assert!(matches!(read_for_desk(&dir.path().join("missing.rs")), CardContent::Unreadable(_)));
        let big = write("big.log", &vec![b'x'; MAX_DESK_BYTES as usize + 1]);
        assert!(matches!(read_for_desk(&big), CardContent::TooLarge(_)));
    }

    #[test]
    fn times_read_as_local_dates() {
        assert_eq!(local_date_time(0, 0), "1970-01-01 00:00");
        // 2026-10-01T10:01:00Z, two hours ahead and five hours behind.
        assert_eq!(local_date_time(1_790_848_860_000, 7200), "2026-10-01 12:01");
        assert_eq!(local_date_time(1_790_848_860_000, -5 * 3600), "2026-10-01 05:01");
        assert_eq!(local_date_time(951_782_400_000, 0), "2000-02-29 00:00");
    }

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../studio_sources/tests/fixtures/agents");
    const MARKER: &str = "SECRET-PROMPT-7f3a";

    /// The A2 fixture (`studio_sources/tests/fixtures/agents`) copied with its placeholders
    /// filled; returns the temp dir, the repo root, the fake ~/.claude, and the index.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, AgentIndex) {
        fn copy_filled(from: &Path, to: &Path, fill: &dyn Fn(&str) -> String) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_filled(&entry.path(), &target, fill);
                } else {
                    std::fs::write(&target, fill(&std::fs::read_to_string(entry.path()).unwrap())).unwrap();
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let root = base.join("repo");
        for sub in ["src", "sub", "wt", "docs"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::create_dir_all(base.join("other")).unwrap();
        std::os::unix::fs::symlink(&root, base.join("link")).unwrap();
        let claude = base.join("claude");
        let fill = |text: &str| {
            text.replace("{ROOT}", root.to_str().unwrap())
                .replace("{LINK}", base.join("link").to_str().unwrap())
                .replace("{OTHER}", base.join("other").to_str().unwrap())
                .replace("{PARENT}", base.to_str().unwrap())
                .replace("{CLAUDE}", claude.to_str().unwrap())
        };
        copy_filled(Path::new(FIXTURE).join("claude").as_path(), &claude, &fill);
        // Files as they are now: lib.rs holds the subagent's edit twice, rel.md and new.rs once,
        // main.rs is gone.
        std::fs::write(root.join("src/lib.rs"), format!("{MARKER}\nx\n{MARKER}\n")).unwrap();
        std::fs::write(root.join("src/new.rs"), format!("x\n{MARKER}")).unwrap();
        std::fs::write(root.join("docs/rel.md"), format!("{MARKER}\n")).unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        let index = studio_sources::index_sessions(&claude, &[root.clone(), root.join("wt")], None, None).unwrap();
        (dir, root, claude, index)
    }

    fn rows(edits: &[PlanEdit]) -> Vec<(&str, &str, String)> {
        edits.iter().map(|e| (e.rel.as_str(), e.kind.as_str(), e.matched.label())).collect()
    }

    #[test]
    fn a_plan_card_lists_steps_with_their_edits_and_the_rest_in_order() {
        let (_dir, root, claude, index) = fixture();
        let s1 = index.sessions.iter().position(|s| s.id == "s1").unwrap();
        let doc = build_plan_doc(&index, s1, 0);
        assert_eq!((doc.slug.as_str(), doc.started.as_str()), ("brave-blue-fox", "2026-10-01 10:00"));
        assert_eq!(doc.branches, ["main"]);
        assert_eq!(doc.worktree, root.display().to_string());
        let PlanBody::Text { doc: text, path, note } = &doc.plan else { panic!("plan text") };
        assert!(text.text.contains(MARKER) && note.is_none());
        assert_eq!(path.as_deref(), Some(claude.join("plans/brave-blue-fox.md").as_path()));

        // Steps: the tasks in creation order, each with the edits made while it was in progress.
        let steps: Vec<_> = doc.steps.iter().map(|s| (s.subject.as_str(), s.state.as_str())).collect();
        assert_eq!(
            steps,
            [
                (format!("{MARKER} task one").as_str(), "completed"),
                (format!("{MARKER} task two").as_str(), "in progress")
            ]
        );
        assert_eq!(
            rows(&doc.steps[0].edits),
            [("src/new.rs", "Write", "line 2".to_string()), ("src/main.rs", "Bash edit", "file gone".to_string())]
        );
        assert_eq!(
            rows(&doc.steps[1].edits),
            [("src/lib.rs", "Edit", "2 candidates".to_string()), ("docs/rel.md", "Edit", "line 1".to_string())]
        );
        // Edits outside any task, in order; reads only counted.
        let rest = rows(&doc.edits);
        assert_eq!(
            rest.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(),
            [("src/lib.rs", "Edit"), ("src/lib.rs", "Edit")]
        );
        assert_eq!(rest[0].2, "line 3", "its structuredPatch still in place");
        assert_eq!(doc.reads, 2);
        assert_eq!(doc.steps[0].edits[0].time, "10:03");
        assert!(doc.steps[0].edits.iter().all(|e| e.path.starts_with(&root)));
    }

    #[test]
    fn a_deleted_plan_is_said_and_shown_from_the_log() {
        let (_dir, _root, claude, index) = fixture();
        let plan = claude.join("plans/brave-blue-fox.md");
        std::fs::remove_file(&plan).unwrap();
        let s1 = index.sessions.iter().position(|s| s.id == "s1").unwrap();
        let doc = build_plan_doc(&index, s1, 0);
        let PlanBody::Text { doc: text, note, .. } = &doc.plan else { panic!("the log's copy") };
        assert_eq!(&*text.text, format!("# {MARKER} plan"));
        assert_eq!(
            note.as_deref(),
            Some(format!("plan file deleted ({}) · shown from the session log", plan.display()).as_str())
        );
        assert_eq!(doc.steps.len(), 2, "the steps are still shown");

        // Found by its file only, and the file is gone.
        let slug_plan = claude.join("plans/calm-green-owl.md");
        std::fs::remove_file(&slug_plan).unwrap();
        let s3 = index.sessions.iter().position(|s| s.id == "s3").unwrap();
        let doc = build_plan_doc(&index, s3, 0);
        assert_eq!(doc.plan.note(), Some(format!("plan file deleted ({})", slug_plan.display())));
        let s6 = index.sessions.iter().position(|s| s.id == "s6").unwrap();
        assert_eq!(build_plan_doc(&index, s6, 0).plan.note().as_deref(), Some("no plan recorded"));
    }

    #[test]
    fn an_open_card_gets_the_sessions_lights_for_its_text() {
        let (_dir, root, _claude, index) = fixture();
        let s1 = index.sessions.iter().position(|s| s.id == "s1").unwrap();
        let files = edits_by_file(&index, s1);
        let lib = &files[&root.join("src/lib.rs")];
        assert_eq!(lib.len(), 3, "two edits of the session and one of its subagent");
        let text = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
        let lights = card_lights(&index, lib, &text);
        let kinds: HashSet<LitKind> = lights.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, HashSet::from([LitKind::Lit, LitKind::Candidate]), "{lights:?}");
        assert!(lights.iter().all(|l| l.source.starts_with("session brave-blue-fox · Edit")));
        // The file changed: each edit is stale where its log put it, within the file (the
        // subagent's edit had no place in its log).
        let stale = card_lights(&index, lib, "a\nb\nc\nd\n");
        assert!(stale.iter().all(|l| l.kind == LitKind::Stale), "{stale:?}");
        let ranges: Vec<_> = stale.iter().map(|l| l.range.clone()).collect();
        assert_eq!(ranges, [3..=4, 4..=4, 4..=4]);
        assert!(!files.contains_key(&root.join("README.md")), "reads light nothing");
    }

    /// Every session's plan card on the real projects (`STUDIO_REAL_REPOS`, colon-separated),
    /// timed; prints the edit rows by match.
    #[test]
    #[ignore]
    fn real_plan_cards() {
        let Ok(repos) = std::env::var("STUDIO_REAL_REPOS") else { return };
        let claude = studio_sources::agents::default_claude_dir().unwrap();
        for repo in repos.split(':').filter(|r| !r.is_empty()) {
            let root = std::fs::canonicalize(repo).unwrap();
            let index = studio_sources::index_sessions(&claude, &[root], None, None).unwrap();
            let started = std::time::Instant::now();
            let (mut worst, mut counts) = (std::time::Duration::ZERO, std::collections::BTreeMap::new());
            for s in 0..index.sessions.len() {
                let t = std::time::Instant::now();
                let doc = build_plan_doc(&index, s, 0);
                worst = worst.max(t.elapsed());
                for e in doc.steps.iter().flat_map(|s| &s.edits).chain(&doc.edits) {
                    let label = match &e.matched {
                        EditMatch::Lit(_) => "lit",
                        EditMatch::Candidates(_) => "candidates",
                        EditMatch::Stale(_) => "stale",
                        EditMatch::Gone => "file gone",
                        EditMatch::Unavailable(_) => "unavailable",
                    };
                    *counts.entry(label).or_insert(0usize) += 1;
                }
            }
            println!(
                "{repo}: {} plan cards in {:?} (worst {worst:?}); edit rows {counts:?}",
                index.sessions.len(),
                started.elapsed()
            );
        }
    }
}
