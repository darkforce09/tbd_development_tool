use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;

/// Describes what the editor buffer represents relative to the file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOrigin {
    /// Buffer holds the entire file as read from disk; `disk_hash` is the hash at load time.
    FullFile { disk_hash: u64 },
    /// Buffer holds a verbatim slice of the file (item or member source).
    Snippet { original: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// File content changed on disk since it was loaded into the editor.
    ChangedOnDisk,
    /// The original snippet no longer appears in the file.
    SnippetNotFound,
    /// The original snippet appears more than once, so the edit target is ambiguous.
    SnippetAmbiguous,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::ChangedOnDisk => write!(f, "file changed on disk since it was opened; reload before saving"),
            EditError::SnippetNotFound => {
                write!(f, "original snippet not found in file; it changed on disk, reload before saving")
            }
            EditError::SnippetAmbiguous => {
                write!(f, "original snippet occurs more than once in file; open the full file to edit")
            }
        }
    }
}

impl std::error::Error for EditError {}

pub fn content_hash(content: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Computes the new full-file content from the current disk content, the editor origin and the edited buffer.
/// Never returns a result that drops content outside the edited region.
pub fn apply_edit(disk: &str, origin: &EditOrigin, buffer: &str) -> Result<String, EditError> {
    let crlf = disk.contains("\r\n");
    match origin {
        EditOrigin::FullFile { disk_hash } => {
            if content_hash(disk) != *disk_hash {
                return Err(EditError::ChangedOnDisk);
            }
            Ok(match_line_endings(buffer, crlf))
        }
        EditOrigin::Snippet { original } => {
            if original.is_empty() {
                return Err(EditError::SnippetNotFound);
            }
            // Snippets are rebuilt from `str::lines`, so they always use LF; match the file's convention.
            let needle = match_line_endings(original, crlf);
            let mut matches = disk.match_indices(needle.as_str());
            let Some((start, _)) = matches.next() else {
                return Err(EditError::SnippetNotFound);
            };
            if matches.next().is_some() {
                return Err(EditError::SnippetAmbiguous);
            }
            let replacement = match_line_endings(buffer, crlf);
            let mut out = String::with_capacity(disk.len() - needle.len() + replacement.len());
            out.push_str(&disk[..start]);
            out.push_str(&replacement);
            out.push_str(&disk[start + needle.len()..]);
            Ok(out)
        }
    }
}

fn match_line_endings(text: &str, crlf: bool) -> String {
    let lf = text.replace("\r\n", "\n");
    if crlf {
        lf.replace('\n', "\r\n")
    } else {
        lf
    }
}

/// Writes `content` to `path` atomically: a temp file in the same directory is written, flushed,
/// given the original permissions and renamed over the target.
pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content)?;
    tmp.as_file().sync_all()?;
    if let Ok(meta) = std::fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n";

    #[test]
    fn snippet_edit_keeps_rest_of_file() {
        let origin = EditOrigin::Snippet { original: "fn b() {\n    2\n}".into() };
        let out = apply_edit(FILE, &origin, "fn b() {\n    3\n}").unwrap();
        assert_eq!(out, "fn a() {\n    1\n}\n\nfn b() {\n    3\n}\n");
    }

    #[test]
    fn snippet_missing_or_ambiguous_is_refused() {
        let gone = EditOrigin::Snippet { original: "fn c() {}".into() };
        assert_eq!(apply_edit(FILE, &gone, "x"), Err(EditError::SnippetNotFound));
        let dup = EditOrigin::Snippet { original: "}\n".into() };
        assert_eq!(apply_edit(FILE, &dup, "x"), Err(EditError::SnippetAmbiguous));
        let empty = EditOrigin::Snippet { original: String::new() };
        assert_eq!(apply_edit(FILE, &empty, "x"), Err(EditError::SnippetNotFound));
    }

    #[test]
    fn snippet_edit_preserves_crlf() {
        let disk = FILE.replace('\n', "\r\n");
        let origin = EditOrigin::Snippet { original: "fn a() {\n    1\n}".into() };
        let out = apply_edit(&disk, &origin, "fn a() {\n    9\n}").unwrap();
        assert_eq!(out, "fn a() {\r\n    9\r\n}\r\n\r\nfn b() {\r\n    2\r\n}\r\n");
    }

    #[test]
    fn full_file_requires_unchanged_disk() {
        let origin = EditOrigin::FullFile { disk_hash: content_hash(FILE) };
        assert_eq!(apply_edit(FILE, &origin, "new").unwrap(), "new");
        assert_eq!(apply_edit("changed", &origin, "new"), Err(EditError::ChangedOnDisk));
    }

    #[test]
    fn atomic_write_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp files left behind");
    }
}
