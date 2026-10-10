//! Links in documents on the Desk: where a link's target points, and which lines it names.
//!
//! Handled: `file:///abs/path` (percent-encoded), paths relative to the document, paths
//! starting with `/` relative to the project's folder, and a `#L10`, `#L10-L20` or `#L10-20`
//! fragment naming lines on any of them. Web links and in-document anchors are not files. `.`
//! and `..` are worked out by name.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

/// A file a link points at, and the lines it names (1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    pub path: PathBuf,
    pub lines: Option<RangeInclusive<usize>>,
}

/// Where `target` (a link as written in a document in `doc_dir`) points, for a project in
/// `root`. `None` for links that are not to a file: web and mail links, other schemes, and
/// anchors inside the document (`#heading`).
pub fn parse_link(target: &str, doc_dir: &Path, root: Option<&Path>) -> Option<LinkTarget> {
    let target = target.trim();
    let (rest, fragment) = match target.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (target, None),
    };
    let rest = rest.split('?').next().unwrap_or_default();
    let path = if let Some(after) = strip_prefix_ignore_case(rest, "file://") {
        // `file:///abs` or `file://localhost/abs`; other hosts are not this machine.
        let abs = after.strip_prefix("localhost").unwrap_or(after);
        if !abs.starts_with('/') {
            return None;
        }
        PathBuf::from(percent_decode(abs)?)
    } else if has_scheme(rest) {
        return None;
    } else if rest.is_empty() {
        // `#heading` or `#L5`: a place in the document itself.
        return None;
    } else {
        match (rest.strip_prefix('/'), root) {
            (Some(rooted), Some(root)) => root.join(rooted),
            _ => doc_dir.join(rest),
        }
    };
    Some(LinkTarget { path: normalize(&path), lines: fragment.and_then(line_range) })
}

/// `path` with `.` and `..` worked out by name (no disk access), so a file reached by two
/// spellings opens one card.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// Whether `s` starts with a URL scheme (`https:`, `mailto:`, ...). A single letter before the
/// colon is a Windows drive, not a scheme.
fn has_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else { return false };
    scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `%XX` escapes decoded; `None` when they do not make UTF-8 text.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// `L10`, `L10-L20` or `L10-20` as lines 10 to 20; anything else (a heading, `L0`, a range
/// that ends before it starts) names no lines.
fn line_range(fragment: &str) -> Option<RangeInclusive<usize>> {
    let number = |s: &str| s.parse::<usize>().ok().filter(|&n| n > 0);
    let rest = fragment.strip_prefix('L')?;
    let (start, end) = match rest.split_once('-') {
        Some((a, b)) => (number(a)?, number(b.strip_prefix('L').unwrap_or(b))?),
        None => {
            let n = number(rest)?;
            (n, n)
        }
    };
    (start <= end).then_some(start..=end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(path: &str, lines: Option<RangeInclusive<usize>>) -> Option<LinkTarget> {
        Some(LinkTarget { path: PathBuf::from(path), lines })
    }

    #[test]
    fn links_point_at_files_and_lines() {
        let doc = Path::new("/p/docs");
        let root = Some(Path::new("/p"));
        let cases: &[(&str, Option<LinkTarget>)] = &[
            // file:// links, percent-encoded, with and without lines.
            ("file:///abs/path/a.rs#L10-L20", link("/abs/path/a.rs", Some(10..=20))),
            ("file:///abs/my%20file.rs", link("/abs/my file.rs", None)),
            ("file:///abs/%C3%A9t%C3%A9.md#L3", link("/abs/été.md", Some(3..=3))),
            ("FILE:///abs/a.rs#L10-20", link("/abs/a.rs", Some(10..=20))),
            ("file://localhost/abs/a.rs", link("/abs/a.rs", None)),
            ("file://otherhost/abs/a.rs", None),
            ("file:///abs/bad%ZZ.rs", link("/abs/bad%ZZ.rs", None)),
            // Relative to the document, and to the project with a leading `/`.
            ("guide.md", link("/p/docs/guide.md", None)),
            ("../README.md#L5", link("/p/README.md", Some(5..=5))),
            ("./a/./b.rs", link("/p/docs/a/b.rs", None)),
            ("/src/lib.rs#L2-L4", link("/p/src/lib.rs", Some(2..=4))),
            ("guide.md?plain=1#L7", link("/p/docs/guide.md", Some(7..=7))),
            // Fragments that name no lines.
            ("guide.md#install", link("/p/docs/guide.md", None)),
            ("a.rs#L20-L10", link("/p/docs/a.rs", None)),
            ("a.rs#L0", link("/p/docs/a.rs", None)),
            ("a.rs#L5-", link("/p/docs/a.rs", None)),
            ("a.rs#Lx", link("/p/docs/a.rs", None)),
            // Not files.
            ("#heading", None),
            ("#L5", None),
            ("", None),
            ("https://example.com/a.rs#L1", None),
            ("mailto:someone@example.com", None),
        ];
        for (target, expected) in cases {
            assert_eq!(&parse_link(target, doc, root), expected, "{target}");
        }
    }

    #[test]
    fn without_a_project_a_leading_slash_is_absolute() {
        assert_eq!(parse_link("/etc/hosts#L1", Path::new("/p"), None), link("/etc/hosts", Some(1..=1)));
    }

    #[test]
    fn a_trailing_percent_is_kept() {
        assert_eq!(percent_decode("/a%2"), Some("/a%2".to_string()));
        assert_eq!(percent_decode("/a%"), Some("/a%".to_string()));
        assert_eq!(percent_decode("/a%2F"), Some("/a/".to_string()));
        assert_eq!(percent_decode("/%FF"), None, "not UTF-8");
    }
}
