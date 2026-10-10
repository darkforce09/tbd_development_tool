//! Resolving `@contract <file>[#<pointer>]` tags to a JSON file and a line in it.
//!
//! The file is found by exact file name among the project files, or by path suffix on a `/` boundary when the tag
//! names a path. Exactly one file whose JSON holds the pointer (RFC 6901, checked with
//! [`serde_json::Value::pointer`]) is Proven; several files with that name are a Possible set; anything else is
//! Unresolved with the reason. Pointers are taken as written: no percent-decoding, and `#/` names the key `""`,
//! not the whole document.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::tags::ContractTag;

/// Project files indexed by file name, for contract lookups.
#[derive(Debug, Clone, Default)]
pub struct ContractFiles {
    by_name: BTreeMap<String, Vec<PathBuf>>,
}

impl ContractFiles {
    /// Indexes `paths` (usually project-relative) by file name. The paths are returned as given.
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for path in paths {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                by_name.entry(name.to_string()).or_default().push(path);
            }
        }
        for list in by_name.values_mut() {
            list.sort();
            list.dedup();
        }
        ContractFiles { by_name }
    }

    /// Files named `file_ref`, or, when it holds a `/`, files whose path ends with it on a `/` boundary. Sorted.
    pub fn candidates(&self, file_ref: &str) -> Vec<PathBuf> {
        let name = file_ref.rsplit('/').next().unwrap_or(file_ref);
        let Some(list) = self.by_name.get(name) else { return Vec::new() };
        if !file_ref.contains('/') {
            return list.clone();
        }
        list.iter()
            .filter(|path| {
                let text = path.to_string_lossy().replace('\\', "/");
                text == file_ref || text.ends_with(&format!("/{file_ref}"))
            })
            .cloned()
            .collect()
    }
}

/// Where a contract tag points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractResolution {
    /// One file, pointer present; `line` is 1-based, where the pointed value's key (or array element) starts.
    Proven {
        file: PathBuf,
        pointer: Option<String>,
        line: usize,
    },
    /// Several files carry the name: sorted.
    Possible(Vec<PathBuf>),
    Unresolved(String),
}

/// The reason a `#/` pointer stays unresolved in a document without an empty key.
pub const EMPTY_KEY_POINTER: &str = "pointer `/` names the empty key; whole document is `#`";

/// Resolves a tag. `read` returns a file's text for a path as stored in `files`.
pub fn resolve_contract(
    tag: &ContractTag,
    files: &ContractFiles,
    read: &dyn Fn(&Path) -> Option<String>,
) -> ContractResolution {
    let mut candidates = files.candidates(&tag.file_ref);
    match candidates.len() {
        0 => return ContractResolution::Unresolved(format!("no file named {}", tag.file_ref)),
        1 => {}
        _ => return ContractResolution::Possible(candidates),
    }
    let file = candidates.remove(0);
    let shown = file.display();
    let Some(text) = read(&file) else { return ContractResolution::Unresolved(format!("cannot read {shown}")) };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(err) => return ContractResolution::Unresolved(format!("{shown} is not JSON: {err}")),
    };
    let pointer = tag.pointer.clone();
    let line = match pointer.as_deref() {
        None | Some("") => 1,
        Some(p) => {
            if value.pointer(p).is_none() {
                // RFC 6901: `/` is the member with the empty key, not the document (that is `#`).
                if p == "/" {
                    return ContractResolution::Unresolved(EMPTY_KEY_POINTER.to_string());
                }
                return ContractResolution::Unresolved(format!("pointer {p} not found in {shown}"));
            }
            pointer_line(&text, p)
        }
    };
    ContractResolution::Proven { file, pointer, line }
}

/// The 1-based line where the value at `pointer` is introduced: its key in an object, its first character in an
/// array. Found by walking the JSON text token by token along the pointer. When the walk cannot place a step
/// (which should not happen for a pointer `serde_json` resolved), the line of the deepest step it placed is
/// returned, and line 1 when it placed none. With duplicate keys the last one wins, as in `serde_json`.
pub fn pointer_line(text: &str, pointer: &str) -> usize {
    let tokens: Vec<String> = pointer.split('/').skip(1).map(|t| t.replace("~1", "/").replace("~0", "~")).collect();
    let b = text.as_bytes();
    let mut pos = skip_ws(b, 0);
    let mut placed = 0usize; // byte offset of the deepest placed key or element
    for token in &tokens {
        let found = match b.get(pos) {
            Some(b'{') => find_member(text, pos, token),
            Some(b'[') => find_element(b, pos, token),
            _ => None,
        };
        let Some((intro, value)) = found else { break };
        placed = intro;
        pos = value;
    }
    text[..placed].bytes().filter(|&c| c == b'\n').count() + 1
}

fn skip_ws(b: &[u8], mut pos: usize) -> usize {
    while pos < b.len() && b[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

/// End (exclusive) of the string starting at `pos` (a `"`).
fn string_end(b: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// End (exclusive) of the value starting at `pos`.
fn value_end(b: &[u8], pos: usize) -> Option<usize> {
    match b.get(pos)? {
        b'"' => string_end(b, pos),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = pos;
            while i < b.len() {
                match b[i] {
                    b'"' => {
                        i = string_end(b, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => {
            let len = b[pos..].iter().position(|c| c.is_ascii_whitespace() || b",}]".contains(c));
            Some(pos + len.unwrap_or(b.len() - pos))
        }
    }
}

/// In the object at `pos`: (offset of the key, offset of the value) of the last member named `key`.
fn find_member(text: &str, pos: usize, key: &str) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let mut i = skip_ws(b, pos + 1);
    let mut found = None;
    while b.get(i) == Some(&b'"') {
        let key_end = string_end(b, i)?;
        let name: String = serde_json::from_str(&text[i..key_end]).ok()?;
        let colon = skip_ws(b, key_end);
        if b.get(colon) != Some(&b':') {
            return found;
        }
        let value = skip_ws(b, colon + 1);
        if name == key {
            found = Some((i, value));
        }
        i = skip_ws(b, value_end(b, value)?);
        if b.get(i) != Some(&b',') {
            break;
        }
        i = skip_ws(b, i + 1);
    }
    found
}

/// In the array at `pos`: the element at `index` (offset of it, twice).
fn find_element(b: &[u8], pos: usize, index: &str) -> Option<(usize, usize)> {
    if index.is_empty() || !index.bytes().all(|c| c.is_ascii_digit()) || (index.len() > 1 && index.starts_with('0')) {
        return None;
    }
    let target: usize = index.parse().ok()?;
    let mut i = skip_ws(b, pos + 1);
    let mut n = 0usize;
    while i < b.len() && b[i] != b']' {
        if n == target {
            return Some((i, i));
        }
        i = skip_ws(b, value_end(b, i)?);
        if b.get(i) != Some(&b',') {
            return None;
        }
        i = skip_ws(b, i + 1);
        n += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{
  "definitions": {
    "Done": {
      "type": "object"
    },
    "a/b": { "x~y": [
      1,
      { "deep": true }
    ] },
    "quote\"key": 1
  }
}"#;

    #[test]
    fn pointer_lines() {
        assert_eq!(pointer_line(DOC, ""), 1);
        assert_eq!(pointer_line(DOC, "/definitions"), 2);
        assert_eq!(pointer_line(DOC, "/definitions/Done"), 3);
        assert_eq!(pointer_line(DOC, "/definitions/Done/type"), 4);
        assert_eq!(pointer_line(DOC, "/definitions/a~1b"), 6);
        assert_eq!(pointer_line(DOC, "/definitions/a~1b/x~0y/1"), 8);
        assert_eq!(pointer_line(DOC, "/definitions/a~1b/x~0y/1/deep"), 8);
        assert_eq!(pointer_line(DOC, "/definitions/quote\"key"), 10);
        assert_eq!(pointer_line(DOC, "/definitions/missing"), 2, "the deepest placed ancestor");
        assert_eq!(pointer_line(r#"{"a": 1, "a": {"b": 2}}"#, "/a/b"), 1);
        assert_eq!(pointer_line("{\"a\": 1,\n\"a\": {\"b\": 2}}", "/a"), 2, "the last duplicate key wins");
    }

    fn tag(file_ref: &str, pointer: Option<&str>) -> ContractTag {
        ContractTag {
            file_ref: file_ref.to_string(),
            pointer: pointer.map(str::to_string),
            partial: false,
            file: PathBuf::from("src/a.rs"),
            line: 1,
            owner: super::super::tags::TagOwner::File,
        }
    }

    #[test]
    fn resolution_tiers() {
        let files = ContractFiles::new(
            ["c/things.json", "x/dup.json", "y/dup.json", "c/bad.json"].into_iter().map(PathBuf::from),
        );
        let read = |p: &Path| match p.to_str()? {
            "c/things.json" => Some(DOC.to_string()),
            "c/bad.json" => Some("{".to_string()),
            _ => None,
        };
        let proven = resolve_contract(&tag("things.json", Some("/definitions/Done")), &files, &read);
        assert_eq!(
            proven,
            ContractResolution::Proven {
                file: PathBuf::from("c/things.json"),
                pointer: Some("/definitions/Done".to_string()),
                line: 3
            }
        );
        let whole = resolve_contract(&tag("c/things.json", None), &files, &read);
        assert!(matches!(whole, ContractResolution::Proven { line: 1, .. }), "{whole:?}");
        assert_eq!(
            resolve_contract(&tag("dup.json", None), &files, &read),
            ContractResolution::Possible(vec![PathBuf::from("x/dup.json"), PathBuf::from("y/dup.json")])
        );
        assert!(matches!(resolve_contract(&tag("y/dup.json", None), &files, &read), ContractResolution::Unresolved(_)));
        assert!(matches!(
            resolve_contract(&tag("x/things.json", None), &files, &read),
            ContractResolution::Unresolved(_)
        ));
        assert!(matches!(resolve_contract(&tag("hings.json", None), &files, &read), ContractResolution::Unresolved(_)));
        assert!(matches!(resolve_contract(&tag("bad.json", None), &files, &read), ContractResolution::Unresolved(_)));
        let missing = resolve_contract(&tag("things.json", Some("/definitions/Nope")), &files, &read);
        assert_eq!(
            missing,
            ContractResolution::Unresolved("pointer /definitions/Nope not found in c/things.json".to_string())
        );
        let slash = resolve_contract(&tag("things.json", Some("/")), &files, &read);
        assert_eq!(slash, ContractResolution::Unresolved(EMPTY_KEY_POINTER.to_string()), "#/ is the key \"\"");
        // A document that has the empty key resolves `#/` to it.
        let files = ContractFiles::new([PathBuf::from("e/empty.json")]);
        let read = |_: &Path| Some("{\n  \"a\": 1,\n  \"\": {}\n}".to_string());
        assert_eq!(
            resolve_contract(&tag("empty.json", Some("/")), &files, &read),
            ContractResolution::Proven { file: PathBuf::from("e/empty.json"), pointer: Some("/".to_string()), line: 3 }
        );
    }
}
