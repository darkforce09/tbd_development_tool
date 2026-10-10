//! Reading one log file: a byte-level skim of each line's top-level fields, and serde (borrowed,
//! everything else ignored) only on the parts of the lines that carry a tool call or its result.
//! Nothing read here outlives the line except paths, ids, numbers and offsets.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::marker::PhantomData;
use std::path::Path;

use memchr::memmem;
use serde::de::value::MapAccessDeserializer;
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::cache::{hash, use_kind, FileFacts, Hunk, PendingBash, TypeCount, UseFact};
use super::discover::{Place, Resolver};
use super::TaskState;

const CHUNK: usize = 1 << 20;

/// Calls `on_line(line, offset)` for each complete line of `path` from byte `start`; stops early
/// when it returns false. Returns the offset just past the last complete line handled.
pub(crate) fn read_lines(path: &Path, start: u64, mut on_line: impl FnMut(&[u8], u64) -> bool) -> std::io::Result<u64> {
    let mut file = std::fs::File::open(path)?;
    if start > 0 {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(start))?;
    }
    let mut buf = vec![0u8; CHUNK];
    let (mut filled, mut base) = (0usize, start);
    loop {
        if filled == buf.len() {
            // One line longer than the buffer.
            buf.resize(buf.len() * 2, 0);
        }
        let n = file.read(&mut buf[filled..])?;
        if n == 0 {
            return Ok(base);
        }
        filled += n;
        let mut pos = 0;
        while let Some(nl) = memchr::memchr(b'\n', &buf[pos..filled]) {
            let line = &buf[pos..pos + nl];
            let keep_going = on_line(line, base + pos as u64);
            pos += nl + 1;
            if !keep_going {
                return Ok(base + pos as u64);
            }
        }
        buf.copy_within(pos..filled, 0);
        filled -= pos;
        base += pos as u64;
    }
}

// ---------------------------------------------------------------------------------------------
// The skim: top-level keys of one JSON object line, values borrowed and left raw.

/// A line's top-level fields. String values are raw (escapes kept, no quotes); `message` and
/// `tool_use_result` are whole JSON values.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Skim<'a> {
    pub kind: Option<&'a [u8]>,
    pub uuid: Option<&'a [u8]>,
    pub timestamp: Option<&'a [u8]>,
    pub cwd: Option<&'a [u8]>,
    pub git_branch: Option<&'a [u8]>,
    pub slug: Option<&'a [u8]>,
    pub message: Option<&'a [u8]>,
    pub tool_use_result: Option<&'a [u8]>,
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// Index of the closing quote of the string whose body starts at `i`.
fn string_end(b: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let q = i + memchr::memchr(b'"', &b[i..])?;
        let mut slashes = 0;
        while q > slashes && b[q - 1 - slashes] == b'\\' {
            slashes += 1;
        }
        if slashes % 2 == 0 {
            return Some(q);
        }
        i = q + 1;
    }
}

/// Index just past the value starting at `i`.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => string_end(b, i + 1).map(|e| e + 1),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => j = string_end(b, j + 1)?,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
                j += 1;
            }
            Some(j)
        }
    }
}

/// The top-level fields of one line, or `None` if it is not a JSON object.
pub(crate) fn skim(b: &[u8]) -> Option<Skim<'_>> {
    let mut s = Skim::default();
    let mut i = skip_ws(b, 0);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i = skip_ws(b, i + 1);
    if b.get(i) == Some(&b'}') {
        return Some(s);
    }
    loop {
        if b.get(i) != Some(&b'"') {
            return None;
        }
        let key_end = string_end(b, i + 1)?;
        let key = &b[i + 1..key_end];
        i = skip_ws(b, key_end + 1);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = skip_ws(b, i + 1);
        let end = value_end(b, i)?;
        let value = &b[i..end];
        let string = || (value.len() >= 2 && value[0] == b'"').then(|| &value[1..value.len() - 1]);
        match key {
            b"type" => s.kind = string(),
            b"uuid" => s.uuid = string(),
            b"timestamp" => s.timestamp = string(),
            b"cwd" => s.cwd = string(),
            b"gitBranch" => s.git_branch = string(),
            b"slug" => s.slug = string(),
            b"message" => s.message = Some(value),
            b"toolUseResult" => s.tool_use_result = Some(value),
            _ => {}
        }
        i = skip_ws(b, end);
        match b.get(i) {
            Some(b',') => i = skip_ws(b, i + 1),
            Some(b'}') => return Some(s),
            _ => return None,
        }
    }
}

/// A raw (still escaped) JSON string body as text.
pub(crate) fn unescape(raw: &[u8]) -> Option<Cow<'_, str>> {
    if memchr::memchr(b'\\', raw).is_none() {
        return std::str::from_utf8(raw).ok().map(Cow::Borrowed);
    }
    let mut quoted = Vec::with_capacity(raw.len() + 2);
    quoted.push(b'"');
    quoted.extend_from_slice(raw);
    quoted.push(b'"');
    serde_json::from_slice::<String>(&quoted).ok().map(Cow::Owned)
}

/// RFC 3339 (`2026-10-10T12:34:56.789Z`, or with an offset) as unix milliseconds.
pub(crate) fn parse_time(t: &[u8]) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let mut n = 0i64;
        for &c in t.get(r)? {
            if !c.is_ascii_digit() {
                return None;
            }
            n = n * 10 + (c - b'0') as i64;
        }
        Some(n)
    };
    if t.len() < 19 || t[4] != b'-' || t[7] != b'-' || !matches!(t[10], b'T' | b't' | b' ') || t[13] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut ms = 0i64;
    if t.get(i) == Some(&b'.') {
        i += 1;
        let mut digits = 0;
        while let Some(c) = t.get(i).filter(|c| c.is_ascii_digit()) {
            if digits < 3 {
                ms = ms * 10 + (c - b'0') as i64;
            }
            digits += 1;
            i += 1;
        }
        for _ in digits..3 {
            ms *= 10;
        }
    }
    let offset_min = match t.get(i) {
        Some(b'Z' | b'z') | None => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            let m = oh * 60 + om;
            if sign == b'+' {
                m
            } else {
                -m
            }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + s - offset_min * 60) * 1000) + ms)
}

// ---------------------------------------------------------------------------------------------
// Serde shapes for the parts that matter, tolerant of anything else.

/// `T` when the value is an object; nothing for other values.
#[derive(Debug, Default)]
struct Lenient<T>(Option<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Lenient<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(|t| Lenient(Some(t)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Lenient(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(Lenient(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(Lenient(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(Lenient(None))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(Lenient(None))
            }
            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(Lenient(None))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(Lenient(None))
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// The objects of an array that read as `T`; empty for other values.
#[derive(Debug)]
struct List<T>(Vec<T>);

impl<T> Default for List<T> {
    fn default() -> Self {
        List(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for List<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = List<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<Lenient<T>>()? {
                    items.extend(item.0);
                }
                Ok(List(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(List::default())
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(List::default())
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(List::default())
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(List::default())
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(List::default())
            }
            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(List::default())
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(List::default())
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// A string (or number, as text); nothing for other values.
#[derive(Debug, Default)]
struct Text<'a>(Option<Cow<'a, str>>);

impl<'de: 'a, 'a> Deserialize<'de> for Text<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Text<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Text(Some(Cow::Borrowed(v))))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Text(Some(Cow::Owned(v.to_string()))))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(Text(Some(Cow::Owned(v.to_string()))))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                Ok(Text(Some(Cow::Owned(v.to_string()))))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(Text(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(Text(None))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(Text(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Text(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Text(None))
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// `true` only for a JSON `true`.
#[derive(Debug, Default, Clone, Copy)]
struct Flag(bool);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Flag;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a bool")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Flag, E> {
                Ok(Flag(v))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_str<E>(self, _: &str) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_unit<E>(self) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Flag, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Flag(false))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Flag, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Flag(false))
            }
        }
        d.deserialize_any(V)
    }
}

/// A non-negative whole number (or a string holding one); nothing for other values.
#[derive(Debug, Default, Clone, Copy)]
struct Num(Option<u64>);

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = Text::deserialize(d)?;
        Ok(Num(text.0.and_then(|t| t.trim().parse::<u64>().ok())))
    }
}

impl Num {
    fn u32(self) -> Option<u32> {
        self.0.map(|n| n.min(u32::MAX as u64) as u32)
    }
}

/// `message` of an assistant or user line: only its `content` items.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Message<'a> {
    #[serde(borrow)]
    content: List<Item<'a>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Item<'a> {
    #[serde(rename = "type", borrow)]
    kind: Text<'a>,
    #[serde(borrow)]
    id: Text<'a>,
    #[serde(borrow)]
    name: Text<'a>,
    #[serde(borrow)]
    input: Lenient<Input<'a>>,
    #[serde(borrow)]
    tool_use_id: Text<'a>,
    is_error: Flag,
}

/// The tool inputs the index reads. `old_string`, `new_string`, `content`, `command`, `plan`,
/// `subject` and `description` are skipped unread.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Input<'a> {
    #[serde(borrow)]
    file_path: Text<'a>,
    offset: Num,
    limit: Num,
    #[serde(rename = "planFilePath", borrow)]
    plan_file_path: Text<'a>,
    #[serde(rename = "taskId", borrow)]
    task_id: Text<'a>,
    #[serde(borrow)]
    status: Text<'a>,
}

/// `toolUseResult`: the numbers of the patch, the files a Bash call edited, a new task's id.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolUseResult<'a> {
    #[serde(rename = "structuredPatch")]
    structured_patch: List<PatchHunk>,
    #[serde(rename = "bashEditDiff", borrow)]
    bash_edit_diff: Lenient<BashEditDiff<'a>>,
    #[serde(borrow)]
    task: Lenient<NewTask<'a>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PatchHunk {
    #[serde(rename = "oldStart")]
    old_start: Num,
    #[serde(rename = "oldLines")]
    old_lines: Num,
    #[serde(rename = "newStart")]
    new_start: Num,
    #[serde(rename = "newLines")]
    new_lines: Num,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BashEditDiff<'a> {
    #[serde(borrow)]
    files: List<BashEditFile<'a>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BashEditFile<'a> {
    #[serde(rename = "filePath", borrow)]
    file_path: Text<'a>,
    hunks: List<PatchHunk>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct NewTask<'a> {
    #[serde(borrow)]
    id: Text<'a>,
}

fn hunks(list: List<PatchHunk>) -> Vec<Hunk> {
    list.0
        .into_iter()
        .map(|h| Hunk {
            old_start: h.old_start.u32().unwrap_or(0),
            old_lines: h.old_lines.u32().unwrap_or(0),
            new_start: h.new_start.u32().unwrap_or(0),
            new_lines: h.new_lines.u32().unwrap_or(0),
        })
        .collect()
}

/// The id of the `n`th file of one Bash call's edit, distinct from the call's own.
pub(crate) fn bash_edit_id(call: u64, n: usize) -> u64 {
    call ^ (n as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

// ---------------------------------------------------------------------------------------------
// One file.

/// What a use waits for in a later line.
#[derive(Debug, Clone, Copy)]
enum Waiting {
    Use(usize),
    Bash(PendingBash),
}

/// Reads lines into a file's [`FileFacts`], from where they stopped.
pub(crate) struct FileParser<'r> {
    pub facts: FileFacts,
    resolver: &'r Resolver,
    /// Task calls are read from main logs only: a subagent's task list is its own.
    tasks: bool,
    waiting: HashMap<u64, Waiting>,
}

impl<'r> FileParser<'r> {
    pub fn new(facts: FileFacts, resolver: &'r Resolver, subagent: bool) -> Self {
        let mut waiting = HashMap::new();
        for (i, u) in facts.uses.iter().enumerate() {
            if u.result.is_none() && u.kind != use_kind::BASH_EDIT {
                waiting.insert(u.id, Waiting::Use(i));
            }
        }
        for b in &facts.pending_bash {
            waiting.insert(b.id, Waiting::Bash(*b));
        }
        Self { facts, resolver, tasks: !subagent, waiting }
    }

    /// Reads `path` from where the facts stopped to its last complete line.
    pub fn read(&mut self, path: &Path, cancel: &dyn Fn() -> bool) -> std::io::Result<()> {
        let start = self.facts.parsed_to;
        let mut n = 0u32;
        let mut cancelled = false;
        let end = read_lines(path, start, |line, offset| {
            n = n.wrapping_add(1);
            if n.is_multiple_of(4096) && cancel() {
                cancelled = true;
                return false;
            }
            self.line(line, offset);
            true
        })?;
        if cancelled {
            return Err(std::io::Error::other("cancelled"));
        }
        self.facts.parsed_to = end;
        self.facts.pending_bash =
            self.waiting.values().filter_map(|w| if let Waiting::Bash(b) = w { Some(*b) } else { None }).collect();
        self.facts.pending_bash.sort_by_key(|b| b.offset);
        Ok(())
    }

    fn line(&mut self, line: &[u8], offset: u64) {
        let f = &mut self.facts;
        if line.iter().all(|c| c.is_ascii_whitespace()) {
            return;
        }
        f.lines += 1;
        let Some(s) = skim(line) else {
            f.bad_lines += 1;
            return;
        };
        if let Some(u) = s.uuid {
            f.uuids.push(hash(u));
        }
        let time = s.timestamp.and_then(parse_time);
        if let Some(t) = time {
            f.started = f.started.min(t);
            f.ended = f.ended.max(t);
        }
        if f.first_cwd.is_none() {
            f.first_cwd = s.cwd.and_then(unescape).map(Cow::into_owned);
        }
        if let Some(branch) = s.git_branch.filter(|b| !b.is_empty()) {
            if !f.branches.iter().any(|b| b.as_bytes() == branch) {
                if let Some(text) = unescape(branch) {
                    f.branches.push(text.into_owned());
                }
            }
        }
        if let Some(slug) = s.slug.filter(|s| !s.is_empty()) {
            if f.slug.as_deref().map(str::as_bytes) != Some(slug) {
                f.slug = unescape(slug).map(Cow::into_owned);
            }
        }
        let len = line.len().min(u32::MAX as usize) as u32;
        let time = time.unwrap_or(0);
        match s.kind {
            Some(b"assistant") => self.assistant(&s, offset, len, time),
            Some(b"user") => self.user(&s, offset, len),
            other => {
                let name = other.unwrap_or(b"(none)");
                match f.unknown_types.iter_mut().find(|t| t.name.as_bytes() == name) {
                    Some(t) => t.count += 1,
                    None => {
                        f.unknown_types.push(TypeCount { name: String::from_utf8_lossy(name).into_owned(), count: 1 })
                    }
                }
            }
        }
    }

    fn assistant(&mut self, s: &Skim, offset: u64, len: u32, time: i64) {
        let Some(message) = s.message else { return };
        if memmem::find(message, b"\"tool_use\"").is_none() {
            return;
        }
        self.facts.parsed_lines += 1;
        let Ok(message) = serde_json::from_slice::<Message>(message) else {
            self.facts.bad_lines += 1;
            return;
        };
        let cwd = s.cwd.and_then(unescape);
        for item in message.content.0 {
            if item.kind.0.as_deref() != Some("tool_use") {
                continue;
            }
            let (Some(id), Some(name)) = (item.id.0, item.name.0) else { continue };
            let id = hash(id.as_bytes());
            if self.waiting.contains_key(&id) {
                // The same call copied again before its result: one is enough.
                continue;
            }
            let input = item.input.0.unwrap_or_default();
            let mut fact = UseFact { id, offset, len, time, status: u8::MAX, ..Default::default() };
            fact.kind = match name.as_ref() {
                "Read" => use_kind::READ,
                "Edit" | "MultiEdit" => use_kind::EDIT,
                "Write" => use_kind::WRITE,
                "ExitPlanMode" => use_kind::PLAN,
                "TaskCreate" if self.tasks => use_kind::TASK_CREATE,
                "TaskUpdate" if self.tasks => use_kind::TASK_UPDATE,
                "Bash" => {
                    self.waiting.insert(id, Waiting::Bash(PendingBash { id, offset, len, time }));
                    continue;
                }
                _ => continue,
            };
            match fact.kind {
                use_kind::READ | use_kind::EDIT | use_kind::WRITE => {
                    let Some(path) = input.file_path.0 else { continue };
                    place(&mut fact, self.resolver.locate(&path, cwd.as_deref()));
                    if fact.kind == use_kind::READ && (input.offset.0.is_some() || input.limit.0.is_some()) {
                        fact.read = Some((input.offset.u32().unwrap_or(0), input.limit.u32().unwrap_or(0)));
                    }
                }
                use_kind::PLAN => fact.path = input.plan_file_path.0.map(Cow::into_owned),
                use_kind::TASK_UPDATE => {
                    fact.task_id = input.task_id.0.map(Cow::into_owned);
                    fact.status =
                        input.status.0.as_deref().map(TaskState::from_status).map_or(u8::MAX, TaskState::code);
                }
                _ => {}
            }
            self.waiting.insert(id, Waiting::Use(self.facts.uses.len()));
            self.facts.uses.push(fact);
        }
    }

    fn user(&mut self, s: &Skim, offset: u64, len: u32) {
        let Some(message) = s.message else { return };
        if self.waiting.is_empty() || memmem::find(message, b"\"tool_result\"").is_none() {
            return;
        }
        // Only lines answering a call the index waits for are parsed.
        let finder = memmem::Finder::new(b"\"tool_use_id\":\"");
        let wanted = finder.find_iter(message).any(|at| {
            let body = &message[at + 15..];
            memchr::memchr(b'"', body).is_some_and(|end| self.waiting.contains_key(&hash(&body[..end])))
        });
        if !wanted {
            return;
        }
        self.facts.parsed_lines += 1;
        let Ok(message) = serde_json::from_slice::<Message>(message) else {
            self.facts.bad_lines += 1;
            return;
        };
        let cwd = s.cwd.and_then(unescape);
        for item in message.content.0 {
            if item.kind.0.as_deref() != Some("tool_result") {
                continue;
            }
            let Some(id) = item.tool_use_id.0 else { continue };
            let id = hash(id.as_bytes());
            let Some(waiting) = self.waiting.remove(&id) else { continue };
            let error = item.is_error.0;
            let result = || -> Option<ToolUseResult> {
                let raw = s.tool_use_result.filter(|r| r.first() == Some(&b'{'))?;
                serde_json::from_slice::<ToolUseResult>(raw).ok()
            };
            match waiting {
                Waiting::Use(i) => {
                    let kind = self.facts.uses[i].kind;
                    let parsed = match kind {
                        use_kind::EDIT | use_kind::WRITE | use_kind::TASK_CREATE if !error => result(),
                        _ => None,
                    };
                    let fact = &mut self.facts.uses[i];
                    fact.result = Some((offset, len));
                    fact.error = error;
                    if let Some(r) = parsed {
                        fact.hunks = hunks(r.structured_patch);
                        if kind == use_kind::TASK_CREATE {
                            fact.task_id = r.task.0.and_then(|t| t.id.0).map(Cow::into_owned);
                        }
                    }
                }
                Waiting::Bash(call) => {
                    if error || s.tool_use_result.is_none_or(|r| memmem::find(r, b"\"bashEditDiff\"").is_none()) {
                        continue;
                    }
                    let Some(r) = result() else { continue };
                    let files = r.bash_edit_diff.0.map(|d| d.files.0).unwrap_or_default();
                    for (n, file) in files.into_iter().enumerate() {
                        let Some(path) = file.file_path.0 else { continue };
                        let mut fact = UseFact {
                            id: bash_edit_id(call.id, n),
                            kind: use_kind::BASH_EDIT,
                            offset: call.offset,
                            len: call.len,
                            time: call.time,
                            result: Some((offset, len)),
                            hunks: hunks(file.hunks),
                            status: u8::MAX,
                            ..Default::default()
                        };
                        place(&mut fact, self.resolver.locate(&path, cwd.as_deref()));
                        self.facts.uses.push(fact);
                    }
                }
            }
        }
    }
}

/// Reads `path` from `start` for the first line whose top-level `cwd` is inside a root. Lines
/// are looked at only when their bytes hold one of `needles` (`"cwd":"<a root's spelling>`, see
/// [`super::discover::cwd_needle`]); returns that `cwd` (if any) and how far it read.
pub(crate) fn later_cwd(
    path: &Path,
    start: u64,
    needles: &[Vec<u8>],
    resolver: &Resolver,
    cancel: &dyn Fn() -> bool,
) -> std::io::Result<(Option<String>, u64)> {
    let finder = memmem::Finder::new(b"\"cwd\":\"");
    let mut found = None;
    let mut n = 0u32;
    let mut cancelled = false;
    let end = read_lines(path, start, |line, _| {
        n = n.wrapping_add(1);
        if n.is_multiple_of(4096) && cancel() {
            cancelled = true;
            return false;
        }
        if !finder.find_iter(line).any(|at| needles.iter().any(|needle| line[at..].starts_with(needle))) {
            return true;
        }
        // A spelling of a root somewhere in the line: decide by its own `cwd`, canonical.
        found = skim(line)
            .and_then(|s| s.cwd)
            .and_then(unescape)
            .filter(|c| resolver.root_of_cwd(c).is_some())
            .map(Cow::into_owned);
        found.is_none()
    })?;
    if cancelled {
        return Err(std::io::Error::other("cancelled"));
    }
    Ok((found, end))
}

/// Puts a located path into `fact`: the path when inside a root, else its folder.
fn place(fact: &mut UseFact, place: Place) {
    match place {
        Place::Inside(p) => fact.path = Some(p),
        Place::Outside(p) => {
            fact.out_of_repo = true;
            fact.outside = Path::new(&p).parent().map(|f| f.to_string_lossy().into_owned());
        }
        Place::Unknown => fact.out_of_repo = true,
    }
}

/// Reads `path` until the first line with a `cwd`; returns it (if any) and how far it read.
pub(crate) fn first_cwd(path: &Path, start: u64) -> std::io::Result<(Option<String>, u64)> {
    let mut found = None;
    let end = read_lines(path, start, |line, _| {
        // Cheap test first: most lines before the first `cwd` are small bookkeeping lines.
        if memmem::find(line, b"\"cwd\"").is_none() {
            return true;
        }
        found = skim(line).and_then(|s| s.cwd).and_then(unescape).map(Cow::into_owned);
        found.is_none()
    })?;
    Ok((found, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skim_reads_top_level_fields_only() {
        let line = br#"{"parentUuid":null,"message":{"type":"message","content":[{"type":"tool_use","cwd":"/nested","uuid":"x"}],"text":"a \"quoted\" {brace"},"type":"assistant","uuid":"u-1","timestamp":"2026-10-10T12:00:00.250Z","cwd":"/repo","gitBranch":"main","slug":"two-words","n":12,"b":true}"#;
        let s = skim(line).unwrap();
        assert_eq!(s.kind, Some(&b"assistant"[..]));
        assert_eq!(s.uuid, Some(&b"u-1"[..]));
        assert_eq!(s.cwd, Some(&b"/repo"[..]));
        assert_eq!(s.git_branch, Some(&b"main"[..]));
        assert_eq!(s.slug, Some(&b"two-words"[..]));
        assert!(s.message.unwrap().starts_with(b"{\"type\":\"message\""));
        assert!(skim(b"not json").is_none());
        assert!(skim(br#"{"a":"unterminated}"#).is_none());
        assert!(skim(br#"{"a":"x\\"}"#).is_some(), "an escaped backslash ends the string");
    }

    #[test]
    fn times_read_as_unix_milliseconds() {
        assert_eq!(parse_time(b"1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time(b"2026-10-10T12:00:00.250Z"), Some(1_791_633_600_250));
        assert_eq!(parse_time(b"2026-10-10T14:00:00.25+02:00"), Some(1_791_633_600_250));
        assert_eq!(parse_time(b"2026-10-10"), None);
    }

    #[test]
    fn odd_shapes_are_ignored_not_fatal() {
        let m: Message = serde_json::from_slice(br#"{"content":"plain text"}"#).unwrap();
        assert!(m.content.0.is_empty());
        let m: Message =
            serde_json::from_slice(br#"{"content":["s",{"type":"tool_use","id":"a","name":"Read","input":[1]}]}"#)
                .unwrap();
        let items = m.content.0;
        assert_eq!(items.len(), 1);
        assert!(items[0].input.0.is_none());
        let i: Input = serde_json::from_slice(br#"{"file_path":"/a","offset":"10","limit":5.5,"taskId":3}"#).unwrap();
        assert_eq!(i.offset.0, Some(10));
        assert_eq!(i.limit.0, None);
        assert_eq!(i.task_id.0.as_deref(), Some("3"));
    }
}
