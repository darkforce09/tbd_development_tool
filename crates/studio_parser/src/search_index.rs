//! The palette's search index: segments of entries (files, symbols, commands, …), matched with
//! [`crate::fuzzy`] and ranked in one total order.
//!
//! The order is score (descending), then kind, name bytes, path bytes, line, key and detail
//! bytes. It never depends on where an entry sits or the order segments arrive in, so the same
//! entries and the same query always give the same list, serially or in parallel.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use studio_graph::{FileMemberNode, Graph, NodeArchetype};

use crate::builder::common::{node_rel_path, slash_path};
use crate::builder::members::build_member_nodes;
use crate::extractor::ExtractedFile;
use crate::fuzzy;
use crate::tree::{DirKind, ProjectTree};

/// Slot of the file segment in a loaded index.
pub const FILES_SLOT: usize = 0;
/// Slot of the symbol segment in a loaded index.
pub const SYMBOLS_SLOT: usize = 1;
/// Slot the viewer fills with its commands and source data (empty in a loaded index).
pub const SOURCES_SLOT: usize = 2;
/// Number of result groups.
pub const GROUP_COUNT: usize = 11;

/// A path match counts this share of its score (numerator / denominator), so names rank first.
const PATH_WEIGHT: (i32, i32) = (3, 4);
/// Added when the query is an entry's whole path, so the full path finds that exact entry first.
const PATH_EXACT: i32 = 50;
/// Entries scanned per parallel task.
const CHUNK: usize = 1024;

/// What an entry is. The order is the fixed group order (D2); folders share the Files group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    File,
    Folder,
    Symbol,
    Command,
    Tool,
    Setting,
    Route,
    Flow,
    Branch,
    Worktree,
    Ticket,
    Session,
}

impl EntryKind {
    pub const ALL: [EntryKind; 12] = [
        EntryKind::File,
        EntryKind::Folder,
        EntryKind::Symbol,
        EntryKind::Command,
        EntryKind::Tool,
        EntryKind::Setting,
        EntryKind::Route,
        EntryKind::Flow,
        EntryKind::Branch,
        EntryKind::Worktree,
        EntryKind::Ticket,
        EntryKind::Session,
    ];

    /// Position of this kind's group (0 = Files).
    pub fn group(self) -> u8 {
        match self {
            EntryKind::File | EntryKind::Folder => 0,
            kind => kind as u8 - 1,
        }
    }

    /// The kind a group is reported under (File for the Files group).
    pub fn group_lead(group: u8) -> EntryKind {
        match group {
            0 => EntryKind::File,
            g => EntryKind::ALL[g as usize + 1],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EntryKind::File => "File",
            EntryKind::Folder => "Folder",
            EntryKind::Symbol => "Symbol",
            EntryKind::Command => "Command",
            EntryKind::Tool => "Tool",
            EntryKind::Setting => "Setting",
            EntryKind::Route => "Route",
            EntryKind::Flow => "Flow",
            EntryKind::Branch => "Branch",
            EntryKind::Worktree => "Worktree",
            EntryKind::Ticket => "Ticket",
            EntryKind::Session => "Session",
        }
    }

    /// Whether the entry's path is matched too (at lower weight).
    fn matches_path(self) -> bool {
        matches!(self, EntryKind::File | EntryKind::Folder | EntryKind::Symbol)
    }
}

/// One searchable thing.
///
/// `key` is opaque to the index but must make `(kind, path, key)` unique, so the order is total:
/// - File, Folder: [`path_key`] of the path.
/// - Symbol: [`symbol_key`]: the member's index in its file card (`member_nodes`) in the low 32
///   bits, a hash of the path in the high 32. Find the card by `path`
///   ([`crate::builder::common::node_rel_path`]) and the member with [`member_index`].
/// - Anything else: an index into the caller's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    /// What is matched first and shown bold.
    pub name: String,
    /// Shown dim (path, signature, value, status…). For files, folders and symbols it starts with
    /// `path`, so path match positions also index it.
    pub detail: String,
    /// Project-relative, `/`-separated.
    pub path: Option<String>,
    /// 1-based.
    pub line: Option<u32>,
    pub key: u64,
}

/// Lowercase text and its char-presence mask, for the prefilter.
#[derive(Debug, Clone, Default)]
struct Folded {
    lower: Box<str>,
    mask: u64,
}

impl Folded {
    fn new(text: &str) -> Self {
        let lower = fuzzy::fold_lower(text);
        Folded { mask: char_mask(&lower), lower: lower.into_boxed_str() }
    }
}

/// One bit per ASCII letter, digit and `_ / . - :`. A candidate missing any of the query's bits
/// cannot match.
fn char_mask(lower: &str) -> u64 {
    lower.bytes().fold(0, |mask, b| {
        let bit = match b {
            b'a'..=b'z' => b - b'a',
            b'0'..=b'9' => 26 + b - b'0',
            b'_' => 36,
            b'/' => 37,
            b'.' => 38,
            b'-' => 39,
            b':' => 40,
            _ => return mask,
        };
        mask | 1 << bit
    })
}

/// No path to match for an entry.
const NO_PATH: u32 = u32::MAX;

/// A batch of entries with their prefilter data. Paths are stored once per distinct path, so a
/// file's many symbols share one path match per query.
#[derive(Debug, Default)]
pub struct Segment {
    entries: Vec<Entry>,
    names: Vec<Folded>,
    paths: Vec<Folded>,
    /// First entry with each distinct path, where its original case is read.
    path_entry: Vec<u32>,
    path_of: Vec<u32>,
    /// Bit per [`EntryKind`] present.
    kinds: u16,
}

impl Segment {
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut distinct: HashMap<&str, u32> = HashMap::new();
        let (mut paths, mut path_entry) = (Vec::new(), Vec::new());
        let mut path_of = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let slot = match e.path.as_deref().filter(|_| e.kind.matches_path()) {
                Some(p) => *distinct.entry(p).or_insert_with(|| {
                    paths.push(Folded::new(p));
                    path_entry.push(i as u32);
                    paths.len() as u32 - 1
                }),
                None => NO_PATH,
            };
            path_of.push(slot);
        }
        let names = entries.iter().map(|e| Folded::new(&e.name)).collect();
        let kinds = entries.iter().fold(0, |k, e| k | 1 << e.kind as u16);
        Segment { entries, names, paths, path_entry, path_of, kinds }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Bytes this segment holds on the heap (entries, folded text, tables).
    pub fn heap_bytes(&self) -> usize {
        let opt = |s: &Option<String>| s.as_ref().map_or(0, String::capacity);
        let entries: usize = self.entries.iter().map(|e| e.name.capacity() + e.detail.capacity() + opt(&e.path)).sum();
        let folded: usize = self.names.iter().chain(&self.paths).map(|f| f.lower.len()).sum();
        entries
            + folded
            + self.entries.capacity() * std::mem::size_of::<Entry>()
            + (self.names.capacity() + self.paths.capacity()) * std::mem::size_of::<Folded>()
            + (self.path_of.capacity() + self.path_entry.capacity()) * 4
    }

    fn has_kind_in(&self, scope: Scope) -> bool {
        EntryKind::ALL.iter().any(|&k| self.kinds & 1 << k as u16 != 0 && scope.includes(k))
    }
}

/// Which kinds a query searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    /// `>`: commands and tools.
    Commands,
    /// `@`: symbols.
    Symbols,
}

impl Scope {
    pub fn includes(self, kind: EntryKind) -> bool {
        match self {
            Scope::All => true,
            Scope::Commands => matches!(kind, EntryKind::Command | EntryKind::Tool),
            Scope::Symbols => kind == EntryKind::Symbol,
        }
    }
}

/// A parsed palette query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub scope: Scope,
    /// Trimmed and lowercased ([`fuzzy::fold_lower`]).
    pub text: String,
}

impl Query {
    /// `>` limits to commands and tools, `@` to symbols. An empty text finds nothing.
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim_start();
        let (scope, rest) = match (raw.strip_prefix('>'), raw.strip_prefix('@')) {
            (Some(rest), _) => (Scope::Commands, rest),
            (_, Some(rest)) => (Scope::Symbols, rest),
            _ => (Scope::All, raw),
        };
        Query { scope, text: fuzzy::fold_lower(rest.trim()) }
    }
}

/// One result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub segment: usize,
    pub entry: usize,
    pub score: i32,
    /// Byte offsets of the matched chars in `name`, or in `path` (and so `detail`) when `in_path`.
    pub positions: Vec<u32>,
    pub in_path: bool,
}

/// One result group: its kind ([`EntryKind::group_lead`]), how many entries matched in all, and
/// the best of them, up to the group's cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub kind: EntryKind,
    pub total: usize,
    pub hits: Vec<Hit>,
}

/// Every searchable segment. Cloning is cheap: segments are shared.
#[derive(Clone, Debug, Default)]
pub struct SearchIndex {
    segments: Vec<Arc<Segment>>,
}

/// A match before its positions are worked out.
#[derive(Clone, Copy)]
struct Cand {
    score: i32,
    segment: u32,
    entry: u32,
    in_path: bool,
}

/// What one scan task found: match counts and the best candidates, per group.
struct Found {
    totals: [usize; GROUP_COUNT],
    best: Vec<Vec<Cand>>,
}

impl SearchIndex {
    /// An index from the loader: files in [`FILES_SLOT`], symbols in [`SYMBOLS_SLOT`] and an empty
    /// [`SOURCES_SLOT`] for the viewer to fill.
    pub fn loaded(files: Arc<Segment>, symbols: Arc<Segment>) -> Self {
        SearchIndex { segments: vec![files, symbols, Arc::new(Segment::default())] }
    }

    /// Adds a segment and returns its slot.
    pub fn push(&mut self, segment: Arc<Segment>) -> usize {
        self.segments.push(segment);
        self.segments.len() - 1
    }

    /// Puts `segment` in `slot`, adding empty slots before it if needed.
    pub fn replace(&mut self, slot: usize, segment: Arc<Segment>) {
        if slot >= self.segments.len() {
            self.segments.resize_with(slot + 1, || Arc::new(Segment::default()));
        }
        self.segments[slot] = segment;
    }

    pub fn segments(&self) -> &[Arc<Segment>] {
        &self.segments
    }

    pub fn entry(&self, hit: &Hit) -> &Entry {
        &self.segments[hit.segment].entries[hit.entry]
    }

    /// Entries per kind, in [`EntryKind::ALL`] order.
    pub fn counts(&self) -> [usize; 12] {
        let mut counts = [0; 12];
        for e in self.segments.iter().flat_map(|s| &s.entries) {
            counts[e.kind as usize] += 1;
        }
        counts
    }

    pub fn heap_bytes(&self) -> usize {
        self.segments.iter().map(|s| s.heap_bytes()).sum()
    }

    /// Groups in kind order, each with its total match count and its best hits, at most
    /// `per_kind(group lead)` of them. Groups with no match are left out.
    pub fn search(&self, query: &Query, per_kind: impl Fn(EntryKind) -> usize) -> Vec<Group> {
        self.run(query, per_kind, true)
    }

    /// [`Self::search`] on one thread; the result is identical.
    pub fn search_serial(&self, query: &Query, per_kind: impl Fn(EntryKind) -> usize) -> Vec<Group> {
        self.run(query, per_kind, false)
    }

    fn run(&self, query: &Query, per_kind: impl Fn(EntryKind) -> usize, parallel: bool) -> Vec<Group> {
        let needle = fuzzy::needle(&query.text);
        if needle.is_empty() {
            return Vec::new();
        }
        let caps: [usize; GROUP_COUNT] = std::array::from_fn(|g| per_kind(EntryKind::group_lead(g as u8)));
        let mut totals = [0; GROUP_COUNT];
        let mut best: Vec<Vec<Cand>> = vec![Vec::new(); GROUP_COUNT];
        for (si, segment) in self.segments.iter().enumerate() {
            if !segment.has_kind_in(query.scope) {
                continue;
            }
            for found in self.scan_segment(si, segment, query.scope, &needle, &caps, parallel) {
                for g in 0..GROUP_COUNT {
                    totals[g] += found.totals[g];
                    best[g].extend_from_slice(&found.best[g]);
                    self.prune(&mut best[g], caps[g]);
                }
            }
        }
        let mut positions = Vec::new();
        (0..GROUP_COUNT)
            .filter(|&g| totals[g] > 0)
            .map(|g| {
                let mut cands = std::mem::take(&mut best[g]);
                cands.sort_unstable_by(|a, b| self.order(a, b));
                cands.truncate(caps[g]);
                let hits = cands.iter().map(|c| self.hit(c, &needle, &mut positions)).collect();
                Group { kind: EntryKind::group_lead(g as u8), total: totals[g], hits }
            })
            .collect()
    }

    /// Scores every entry of one segment, in chunks.
    fn scan_segment(
        &self,
        si: usize,
        segment: &Segment,
        scope: Scope,
        needle: &str,
        caps: &[usize; GROUP_COUNT],
        parallel: bool,
    ) -> Vec<Found> {
        let nmask = char_mask(needle);
        let path_score = |slot: usize| path_score(segment, slot, needle, nmask);
        let path_scores: Vec<Option<i32>> = if parallel {
            (0..segment.paths.len()).into_par_iter().map(path_score).collect()
        } else {
            (0..segment.paths.len()).map(path_score).collect()
        };
        let scan = |(ci, chunk): (usize, &[Entry])| {
            let mut found = Found { totals: [0; GROUP_COUNT], best: vec![Vec::new(); GROUP_COUNT] };
            for (offset, entry) in chunk.iter().enumerate() {
                let i = ci * CHUNK + offset;
                if !scope.includes(entry.kind) {
                    continue;
                }
                let Some((score, in_path)) = entry_score(segment, i, needle, nmask, &path_scores) else { continue };
                let g = entry.kind.group() as usize;
                found.totals[g] += 1;
                if caps[g] > 0 {
                    found.best[g].push(Cand { score, segment: si as u32, entry: i as u32, in_path });
                    self.prune(&mut found.best[g], caps[g]);
                }
            }
            found
        };
        if parallel {
            segment.entries.par_chunks(CHUNK).enumerate().map(scan).collect()
        } else {
            segment.entries.chunks(CHUNK).enumerate().map(scan).collect()
        }
    }

    /// Keeps the best `cap` candidates once the list holds twice that many.
    fn prune(&self, cands: &mut Vec<Cand>, cap: usize) {
        if cap == 0 {
            cands.clear();
        } else if cands.len() >= cap.saturating_mul(2) {
            cands.select_nth_unstable_by(cap - 1, |a, b| self.order(a, b));
            cands.truncate(cap);
        }
    }

    /// The total order: score (descending), kind, name, path, line, key, detail. Only fully equal
    /// entries fall through to their position.
    fn order(&self, a: &Cand, b: &Cand) -> Ordering {
        let ea = &self.segments[a.segment as usize].entries[a.entry as usize];
        let eb = &self.segments[b.segment as usize].entries[b.entry as usize];
        b.score
            .cmp(&a.score)
            .then(ea.kind.cmp(&eb.kind))
            .then_with(|| ea.name.as_bytes().cmp(eb.name.as_bytes()))
            .then_with(|| ea.path.as_deref().map(str::as_bytes).cmp(&eb.path.as_deref().map(str::as_bytes)))
            .then(ea.line.cmp(&eb.line))
            .then(ea.key.cmp(&eb.key))
            .then_with(|| ea.detail.as_bytes().cmp(eb.detail.as_bytes()))
            .then(a.in_path.cmp(&b.in_path))
            .then((a.segment, a.entry).cmp(&(b.segment, b.entry)))
    }

    /// A final hit, with the matched positions of the field that won.
    fn hit(&self, c: &Cand, needle: &str, scratch: &mut Vec<u32>) -> Hit {
        let entry = &self.segments[c.segment as usize].entries[c.entry as usize];
        let text = if c.in_path { entry.path.as_deref().unwrap_or_default() } else { &entry.name };
        fuzzy::score(needle, text, &fuzzy::fold_lower(text), Some(scratch));
        Hit {
            segment: c.segment as usize,
            entry: c.entry as usize,
            score: c.score,
            positions: scratch.clone(),
            in_path: c.in_path,
        }
    }
}

/// Weighted score of a path match, plus [`PATH_EXACT`] when the query is the whole path.
fn path_score(segment: &Segment, slot: usize, needle: &str, nmask: u64) -> Option<i32> {
    let path = &segment.paths[slot];
    if path.mask & nmask != nmask {
        return None;
    }
    let text = segment.entries[segment.path_entry[slot] as usize].path.as_deref().unwrap_or_default();
    let raw = fuzzy::score(needle, text, &path.lower, None)?;
    let exact = if *path.lower == *needle { PATH_EXACT } else { 0 };
    Some((raw * PATH_WEIGHT.0).div_euclid(PATH_WEIGHT.1) + exact)
}

/// The better of an entry's name and path scores; the name wins a tie. A whole-path match keeps
/// its [`PATH_EXACT`] bonus even when the name scores higher.
fn entry_score(
    segment: &Segment,
    i: usize,
    needle: &str,
    nmask: u64,
    path_scores: &[Option<i32>],
) -> Option<(i32, bool)> {
    let name = &segment.names[i];
    let by_name = (name.mask & nmask == nmask)
        .then(|| fuzzy::score(needle, &segment.entries[i].name, &name.lower, None))
        .flatten();
    let slot = segment.path_of[i];
    let by_path = (slot != NO_PATH).then(|| path_scores[slot as usize]).flatten();
    let exact_path = by_path.is_some() && *segment.paths[slot as usize].lower == *needle;
    match (by_name, by_path) {
        (Some(n), Some(p)) if p > n => Some((p, true)),
        (Some(n), _) if exact_path => Some((n + PATH_EXACT, false)),
        (Some(n), _) => Some((n, false)),
        (None, Some(p)) => Some((p, true)),
        (None, None) => None,
    }
}

/// [`Entry::key`] of a file or folder: a 64-bit FNV-1a hash of its path.
pub fn path_key(path: &str) -> u64 {
    path.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// [`Entry::key`] of a symbol: its file's path hash (high 32 bits) and its member index (low 32).
pub fn symbol_key(path: &str, member: usize) -> u64 {
    (path_key(path) >> 32) << 32 | member as u64 & 0xffff_ffff
}

/// The member index inside a symbol's [`Entry::key`].
pub fn member_index(key: u64) -> usize {
    (key & 0xffff_ffff) as usize
}

/// What the Files group lists (D1): every file the disk scan sees, except single files git
/// ignores, and one entry per heavy or ignored folder, never its contents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileListing {
    /// Project-relative `/` paths, sorted.
    pub files: Vec<String>,
    /// (project-relative path, why it is collapsed), sorted.
    pub folders: Vec<(String, String)>,
}

impl FileListing {
    /// From a disk scan of the folder `prefix` (project-relative, "" for the project root).
    pub fn from_tree(tree: &ProjectTree, prefix: &str) -> Self {
        let join = |rel: &Path| match (prefix, slash_path(rel)) {
            ("", rel) => rel,
            (prefix, rel) => format!("{prefix}/{rel}"),
        };
        let ignored: std::collections::HashSet<&PathBuf> = tree.ignored_files.iter().collect();
        let files = tree.files.iter().filter(|f| !ignored.contains(&f.rel)).map(|f| join(&f.rel)).collect();
        let folders = tree
            .dirs
            .iter()
            .filter_map(|d| match &d.kind {
                DirKind::Heavy(info) => Some((join(&d.rel), info.reason.label().to_string())),
                _ => None,
            })
            .collect();
        Self::sorted(files, folders)
    }

    /// From a Files graph, for the cache path, which has no scan: its file cards and its unloaded
    /// folders. `ignored` are the single files git ignores ([`crate::tree::git_ignored`]).
    pub fn from_graph(graph: &Graph, ignored: &[PathBuf]) -> Self {
        let ignored: std::collections::HashSet<String> = ignored.iter().map(|p| slash_path(p)).collect();
        let files = graph.nodes.values().filter_map(node_rel_path).filter(|p| !ignored.contains(p)).collect();
        let folders = graph
            .clusters
            .iter()
            .filter_map(|c| Some((c.id.strip_prefix("dir:")?.to_string(), c.lazy.as_ref()?.reason.clone())))
            .collect();
        Self::sorted(files, folders)
    }

    fn sorted(mut files: Vec<String>, mut folders: Vec<(String, String)>) -> Self {
        files.sort();
        folders.sort();
        FileListing { files, folders }
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The file segment (D1): one File entry per file (name = file name, detail = path) and one Folder
/// entry per heavy or ignored folder (detail = "path · reason").
pub fn file_segment(listing: &FileListing) -> Segment {
    let files = listing.files.iter().map(|p| Entry {
        kind: EntryKind::File,
        name: basename(p).to_string(),
        detail: p.clone(),
        path: Some(p.clone()),
        line: None,
        key: path_key(p),
    });
    let folders = listing.folders.iter().map(|(p, reason)| Entry {
        kind: EntryKind::Folder,
        name: basename(p).to_string(),
        detail: format!("{p} · {reason}"),
        path: Some(p.clone()),
        line: None,
        key: path_key(p),
    });
    Segment::new(files.chain(folders).collect())
}

/// The symbol segment (D2): one entry per named member of every file card (detail = "path:line").
pub fn symbol_segment(graph: &Graph) -> Segment {
    let mut entries = Vec::new();
    for node in graph.nodes.values() {
        if let Some(path) = node_rel_path(node) {
            push_symbols(&path, &node.member_nodes, &mut entries);
        }
    }
    Segment::new(entries)
}

/// The symbol segment of a folder loaded on demand, from its parsed files, before they are in the
/// graph. Member indexes match the cards [`crate::materialize_folder`] makes from the same files.
pub fn folder_symbol_segment(prefix: &str, parsed: &[ExtractedFile]) -> Segment {
    let mut entries = Vec::new();
    for file in parsed {
        let rel = slash_path(&file.relative_path);
        let path = if prefix.is_empty() { rel } else { format!("{prefix}/{rel}") };
        push_symbols(&path, &build_member_nodes(file), &mut entries);
    }
    Segment::new(entries)
}

fn push_symbols(path: &str, members: &[FileMemberNode], out: &mut Vec<Entry>) {
    for (i, m) in members.iter().enumerate().filter(|(_, m)| is_named_symbol(m)) {
        out.push(Entry {
            kind: EntryKind::Symbol,
            name: m.name.clone(),
            detail: format!("{path}:{}", m.line_number),
            path: Some(path.to_string()),
            line: Some(m.line_number as u32),
            key: symbol_key(path, i),
        });
    }
}

/// Leaves out what is not a named symbol: documentation link rows, fenced code blocks and
/// unnamed members.
fn is_named_symbol(m: &FileMemberNode) -> bool {
    !m.name.trim().is_empty() && m.archetype != NodeArchetype::Link && !m.id.starts_with("fn:block:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: EntryKind, name: &str, path: Option<&str>, key: u64) -> Entry {
        let p = path.map(str::to_string);
        Entry { kind, name: name.into(), detail: p.clone().unwrap_or_default(), path: p, line: None, key }
    }

    #[test]
    fn kinds_keep_the_fixed_group_order() {
        let groups: Vec<u8> = EntryKind::ALL.iter().map(|k| k.group()).collect();
        assert_eq!(groups, [0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        for g in 0..GROUP_COUNT as u8 {
            assert_eq!(EntryKind::group_lead(g).group(), g);
        }
    }

    #[test]
    fn query_prefixes_pick_the_scope() {
        assert_eq!(Query::parse("  > Files "), Query { scope: Scope::Commands, text: "files".into() });
        assert_eq!(Query::parse("@Parse"), Query { scope: Scope::Symbols, text: "parse".into() });
        assert_eq!(Query::parse(" HUD "), Query { scope: Scope::All, text: "hud".into() });
        let index = SearchIndex::loaded(Arc::new(file_segment(&FileListing::default())), Arc::default());
        assert!(index.search(&Query::parse("   "), |_| 8).is_empty());
        assert!(index.search(&Query::parse(">"), |_| 8).is_empty());
    }

    #[test]
    fn the_prefilter_mask_never_rejects_a_match() {
        let names = ["src/ui/hud.rs", "Größe_2.c", "a-b.c:d", "x"];
        for name in names {
            let lower = fuzzy::fold_lower(name);
            for q in ["hud", "u/h", "ö2", "-b.c:", "x", "2.c"] {
                let needle = fuzzy::needle(q);
                if fuzzy::score(&needle, name, &lower, None).is_some() {
                    assert_eq!(char_mask(&lower) & char_mask(&needle), char_mask(&needle), "{q} in {name}");
                }
            }
        }
    }

    #[test]
    fn totals_count_every_match_and_caps_only_trim_the_list() {
        let entries = (0..30).map(|i| entry(EntryKind::Command, &format!("cmd {i:02}"), None, i)).collect();
        let mut index = SearchIndex::default();
        index.push(Arc::new(Segment::new(entries)));
        let groups = index.search(&Query::parse("cmd"), |_| 5);
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].kind, groups[0].total, groups[0].hits.len()), (EntryKind::Command, 30, 5));
        let zero = index.search(&Query::parse("cmd"), |_| 0);
        assert_eq!((zero[0].total, zero[0].hits.len()), (30, 0));
    }

    #[test]
    fn a_path_match_finds_a_file_and_marks_its_positions() {
        let listing = FileListing { files: vec!["crates/ui/hud.rs".into(), "hud.rs".into()], folders: vec![] };
        let index = SearchIndex::loaded(Arc::new(file_segment(&listing)), Arc::default());
        let groups = index.search(&Query::parse("ui/hud"), |_| 8);
        let hit = &groups[0].hits[0];
        assert_eq!(index.entry(hit).path.as_deref(), Some("crates/ui/hud.rs"));
        assert!(hit.in_path);
        assert_eq!(hit.positions, vec![7, 8, 9, 10, 11, 12]);
        assert_eq!(groups[0].total, 1);
    }

    #[test]
    fn symbol_keys_keep_the_member_index() {
        let key = symbol_key("src/lib.rs", 7);
        assert_eq!(member_index(key), 7);
        assert_ne!(key, symbol_key("src/main.rs", 7));
    }
}
