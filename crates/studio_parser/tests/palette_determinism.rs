//! E3: the same entries and query always give the same list: across rebuilds, with the entries
//! inserted in another order, with the segments arriving in another order, and serial vs parallel.
//! AM2: a group's total counts every match, and a full path always finds its file first.

use std::sync::Arc;

use studio_parser::search_index::{path_key, symbol_key};
use studio_parser::{file_segment, Entry, EntryKind, FileListing, Group, Query, SearchIndex, Segment};

/// A tiny linear congruential generator (Knuth's MMIX constants), so the test needs no crate.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const PARTS: &[&str] = &[
    "hud",
    "objective",
    "parse",
    "file",
    "tree",
    "load",
    "Manager",
    "Component",
    "render",
    "ui",
    "game",
    "scr",
    "SCR",
    "Base",
    "io",
    "net",
    "config",
    "état",
    "größe",
    "x",
    "data",
    "Layout",
    "event",
    "handler",
    "a1",
    "v2",
];
const SEPS: &[&str] = &["_", "", "-", ".", ""];
const DIRS: &[&str] = &["src", "crates", "scripts", "Game", "ui", "web", "docs", "lib", "core", "été"];
const EXTS: &[&str] = &["rs", "ts", "c", "md", "toml", "tsx"];

fn word(rng: &mut Lcg) -> String {
    let n = 1 + rng.below(3);
    (0..n)
        .map(|i| if i == 0 { rng.pick(PARTS).to_string() } else { format!("{}{}", rng.pick(SEPS), rng.pick(PARTS)) })
        .collect()
}

fn path(rng: &mut Lcg) -> String {
    let depth = rng.below(4);
    let mut parts: Vec<String> = (0..depth).map(|_| rng.pick(DIRS).to_string()).collect();
    parts.push(format!("{}.{}", word(rng), rng.pick(EXTS)));
    parts.join("/")
}

/// 2,000 entries of every kind, with near-duplicates (the generator repeats itself often).
fn entries(seed: u64) -> Vec<Entry> {
    let mut rng = Lcg(seed);
    (0..2000)
        .map(|i| {
            let kind = EntryKind::ALL[rng.below(EntryKind::ALL.len())];
            let p = path(&mut rng);
            let name = match kind {
                EntryKind::File | EntryKind::Folder => p.rsplit('/').next().unwrap().to_string(),
                _ => word(&mut rng),
            };
            let line = (rng.below(3) > 0).then(|| rng.below(500) as u32 + 1);
            let key = match kind {
                EntryKind::File | EntryKind::Folder => path_key(&p),
                EntryKind::Symbol => symbol_key(&p, i),
                _ => i as u64,
            };
            let path = matches!(kind, EntryKind::File | EntryKind::Folder | EntryKind::Symbol).then(|| p.clone());
            Entry { kind, name, detail: p, path, line, key }
        })
        .collect()
}

/// 200 queries: pieces of names and paths, scoped ones, and some that match nothing.
fn queries(seed: u64) -> Vec<String> {
    let mut rng = Lcg(seed);
    (0..200)
        .map(|_| {
            let base = match rng.below(5) {
                0 => path(&mut rng),
                1 => word(&mut rng).chars().step_by(2).collect(),
                _ => word(&mut rng),
            };
            let cut: String = base.chars().take(1 + rng.below(8)).collect();
            match rng.below(6) {
                0 => format!(">{cut}"),
                1 => format!("@{cut}"),
                2 => cut.to_uppercase(),
                _ => cut,
            }
        })
        .collect()
}

/// An index of `entries` split into `parts` segments, pushed in `order`.
fn index_of(entries: &[Entry], parts: usize, order: &[usize]) -> SearchIndex {
    let chunk = entries.len().div_ceil(parts);
    let segments: Vec<Vec<Entry>> = entries.chunks(chunk).map(<[Entry]>::to_vec).collect();
    let mut index = SearchIndex::default();
    for &i in order {
        index.push(Arc::new(Segment::new(segments[i].clone())));
    }
    index
}

/// What a result shows, without segment or entry positions.
type Shown = Vec<(EntryKind, usize, Vec<(Entry, i32, Vec<u32>, bool)>)>;

fn shown(index: &SearchIndex, groups: &[Group]) -> Shown {
    groups
        .iter()
        .map(|g| {
            let hits =
                g.hits.iter().map(|h| (index.entry(h).clone(), h.score, h.positions.clone(), h.in_path)).collect();
            (g.kind, g.total, hits)
        })
        .collect()
}

fn caps(kind: EntryKind) -> usize {
    match kind {
        EntryKind::File | EntryKind::Symbol => 8,
        EntryKind::Tool => 50,
        _ => 5,
    }
}

#[test]
fn the_same_entries_and_query_always_give_the_same_list() {
    let base = entries(7);
    let mut reversed = base.clone();
    reversed.reverse();
    let builds = [
        index_of(&base, 4, &[0, 1, 2, 3]),
        index_of(&base, 4, &[0, 1, 2, 3]),
        index_of(&base, 4, &[3, 1, 0, 2]),
        index_of(&reversed, 4, &[0, 1, 2, 3]),
        index_of(&reversed, 3, &[2, 0, 1]),
        index_of(&base, 1, &[0]),
    ];
    let mut matched = 0;
    for q in queries(11) {
        let query = Query::parse(&q);
        let expected = shown(&builds[0], &builds[0].search(&query, caps));
        matched += usize::from(!expected.is_empty());
        for (b, index) in builds.iter().enumerate() {
            assert_eq!(shown(index, &index.search(&query, caps)), expected, "{q:?}: build {b}");
            assert_eq!(shown(index, &index.search_serial(&query, caps)), expected, "{q:?}: build {b}, serial");
        }
    }
    assert!(matched > 100, "most queries match something ({matched})");
}

#[test]
fn totals_count_every_match_whatever_the_caps() {
    let index = index_of(&entries(3), 2, &[1, 0]);
    for q in queries(5) {
        let query = Query::parse(&q);
        let capped: Vec<(EntryKind, usize)> = index.search(&query, caps).iter().map(|g| (g.kind, g.total)).collect();
        let all = index.search(&query, |_| usize::MAX);
        let uncapped: Vec<(EntryKind, usize)> = all.iter().map(|g| (g.kind, g.total)).collect();
        assert_eq!(capped, uncapped, "{q:?}");
        for g in &all {
            assert_eq!(g.hits.len(), g.total, "{q:?}: an uncapped group lists every match");
        }
    }
}

/// Where the file at `path` ranks in the Files group when the query is that full path.
fn rank_of_full_path(index: &SearchIndex, path: &str) -> Option<usize> {
    let groups = index.search(&Query::parse(path), |_| 8);
    let files = groups.iter().find(|g| g.kind == EntryKind::File)?;
    files.hits.iter().position(|h| {
        let e = index.entry(h);
        e.kind == EntryKind::File && e.path.as_deref() == Some(path)
    })
}

#[test]
fn a_full_path_ranks_its_file_first() {
    // Generated files: unique paths, many sharing names.
    let mut seen = std::collections::BTreeSet::new();
    let files: Vec<String> = entries(13)
        .into_iter()
        .filter(|e| e.kind == EntryKind::File)
        .filter_map(|e| e.path)
        .filter(|p| seen.insert(p.clone()))
        .collect();
    let listing = FileListing { files: files.clone(), folders: vec![] };
    let index = SearchIndex::loaded(Arc::new(file_segment(&listing)), Arc::default());
    let mut rng = Lcg(17);
    for _ in 0..20 {
        let p = &files[rng.below(files.len())];
        assert_eq!(rank_of_full_path(&index, p), Some(0), "{p}");
    }

    // This repository's real files.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();
    let tree = studio_parser::tree::scan_tree(root, Default::default());
    let listing = FileListing::from_tree(&tree, "");
    let index = SearchIndex::loaded(Arc::new(file_segment(&listing)), Arc::default());
    for _ in 0..20 {
        let p = &listing.files[rng.below(listing.files.len())];
        assert_eq!(rank_of_full_path(&index, p), Some(0), "{p}");
    }
}
