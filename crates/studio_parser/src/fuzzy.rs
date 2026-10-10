//! Deterministic fuzzy matching for the command palette: a case-insensitive subsequence match,
//! scored by where the matched characters sit. Pure functions, no I/O.
//!
//! A score is the sum of, for every matched character, [`MATCH`] plus its bonuses
//! ([`CONTIGUOUS`], [`WORD_START`] or [`CAMEL_HUMP`], [`BASENAME`]), the gap penalties between
//! matched characters ([`GAP_OPEN`], [`GAP_EXTEND`], at most [`GAP_MAX`] per gap), the late-start
//! penalty ([`LATE_START`], at most [`LATE_START_MAX`]), and [`PREFIX`] / [`EXACT`] for a basename
//! prefix or the whole candidate. ASCII candidates are matched as bytes; anything else is matched
//! char by char, so a UTF-8 character is never split.

/// Every matched character.
pub const MATCH: i32 = 16;
/// A matched character right after the previous matched one (a contiguous run).
pub const CONTIGUOUS: i32 = 12;
/// A matched character at the start of a word: the candidate start, or after `/ _ - . :` or a space.
pub const WORD_START: i32 = 20;
/// A matched uppercase letter right after a lowercase letter or a digit (a camelCase hump). Not
/// added on top of [`WORD_START`].
pub const CAMEL_HUMP: i32 = 16;
/// A matched character inside the basename (after the last `/`; the whole candidate when it has none).
pub const BASENAME: i32 = 6;
/// The query matches the basename's first characters contiguously (the candidate's, when it has no
/// `/`).
pub const PREFIX: i32 = 40;
/// The candidate is the query (case-insensitively).
pub const EXACT: i32 = 100;
/// Opening a gap between two matched characters.
pub const GAP_OPEN: i32 = -3;
/// Each skipped character in a gap after the first.
pub const GAP_EXTEND: i32 = -1;
/// The most a single gap costs.
pub const GAP_MAX: i32 = -12;
/// Each character before the first match.
pub const LATE_START: i32 = -1;
/// The most a late first match costs.
pub const LATE_START_MAX: i32 = -15;
/// How many places the first query character is tried at (its first occurrence, then word starts,
/// humps and the first one in the basename).
const MAX_STARTS: usize = 6;

/// A scored match: the score and the byte offset of each matched character in the candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<u32>,
}

/// Lowercases `s` one char for one char (a char whose lowercase is several chars is kept), so char
/// positions in the result are char positions in `s`.
pub fn fold_lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    s.chars().map(fold_char).collect()
}

fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// The query as matched: folded to lowercase, whitespace removed (`objective hud` matches like
/// `objectivehud`).
pub fn needle(query: &str) -> String {
    fold_lower(query).chars().filter(|c| !c.is_whitespace()).collect()
}

/// Matches `query` against `candidate`. `None` when the query is empty or not a subsequence.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<Match> {
    let mut positions = Vec::new();
    let score = score(&needle(query), candidate, &fold_lower(candidate), Some(&mut positions))?;
    Some(Match { score, positions })
}

/// Scores a prepared [`needle`] against `candidate`, whose [`fold_lower`] is `lower`. Fills
/// `positions` (cleared first) with the matched byte offsets when given.
pub fn score(needle: &str, candidate: &str, lower: &str, positions: Option<&mut Vec<u32>>) -> Option<i32> {
    if needle.is_empty() {
        return None;
    }
    if needle.is_ascii() && lower.is_ascii() {
        return best(needle.as_bytes(), lower.as_bytes(), candidate.as_bytes(), positions);
    }
    let n: Vec<char> = needle.chars().collect();
    let l: Vec<char> = lower.chars().collect();
    let o: Vec<char> = candidate.chars().collect();
    match positions {
        None => best(&n, &l, &o, None),
        Some(out) => {
            let mut chars = Vec::new();
            let s = best(&n, &l, &o, Some(&mut chars))?;
            let offsets: Vec<u32> = candidate.char_indices().map(|(b, _)| b as u32).collect();
            out.clear();
            out.extend(chars.iter().map(|&i| offsets[i as usize]));
            Some(s)
        }
    }
}

/// One matchable unit: a byte of ASCII text, or a char.
trait Unit: Copy + Eq {
    fn is_sep(self) -> bool;
    fn is_slash(self) -> bool;
    fn is_upper(self) -> bool;
    fn is_lower_or_digit(self) -> bool;
}

impl Unit for u8 {
    fn is_sep(self) -> bool {
        matches!(self, b'/' | b'_' | b'-' | b'.' | b':' | b' ')
    }
    fn is_slash(self) -> bool {
        self == b'/'
    }
    fn is_upper(self) -> bool {
        self.is_ascii_uppercase()
    }
    fn is_lower_or_digit(self) -> bool {
        self.is_ascii_lowercase() || self.is_ascii_digit()
    }
}

impl Unit for char {
    fn is_sep(self) -> bool {
        matches!(self, '/' | '_' | '-' | '.' | ':') || self.is_whitespace()
    }
    fn is_slash(self) -> bool {
        self == '/'
    }
    fn is_upper(self) -> bool {
        self.is_uppercase()
    }
    fn is_lower_or_digit(self) -> bool {
        self.is_lowercase() || self.is_numeric()
    }
}

/// The best greedy match over a few start points of the first query unit. Ties keep the earliest
/// start.
fn best<T: Unit>(needle: &[T], lower: &[T], orig: &[T], positions: Option<&mut Vec<u32>>) -> Option<i32> {
    if needle.len() > lower.len() {
        return None;
    }
    let base = lower.iter().rposition(|u| u.is_slash()).map_or(0, |i| i + 1);
    let mut best: Option<(i32, usize)> = None;
    for start in starts(needle[0], lower, orig, base) {
        let Some(s) = run(needle, lower, orig, base, start, None) else { break };
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, start));
        }
    }
    let (score, start) = best?;
    if let Some(out) = positions {
        run(needle, lower, orig, base, start, Some(out));
    }
    Some(score)
}

/// Where the first query unit is tried, in order: its first occurrence, then occurrences at a word
/// start or hump, and the first one inside the basename.
fn starts<'a, T: Unit>(first: T, lower: &'a [T], orig: &'a [T], base: usize) -> impl Iterator<Item = usize> + 'a {
    let mut seen_any = false;
    let mut seen_base = false;
    (0..lower.len())
        .filter(move |&i| {
            if lower[i] != first {
                return false;
            }
            let keep = !seen_any || boundary(orig, i) > 0 || (i >= base && !seen_base);
            if keep {
                seen_any = true;
                seen_base |= i >= base;
            }
            keep
        })
        .take(MAX_STARTS)
}

/// Word-start or hump bonus of the unit at `i`.
fn boundary<T: Unit>(orig: &[T], i: usize) -> i32 {
    if i == 0 || orig[i - 1].is_sep() {
        WORD_START
    } else if orig[i].is_upper() && orig[i - 1].is_lower_or_digit() {
        CAMEL_HUMP
    } else {
        0
    }
}

/// Scores the greedy match that takes the first query unit at `start` and every next one at its
/// earliest place after the previous. `None` when the rest does not fit.
fn run<T: Unit>(
    needle: &[T],
    lower: &[T],
    orig: &[T],
    base: usize,
    start: usize,
    mut positions: Option<&mut Vec<u32>>,
) -> Option<i32> {
    if let Some(out) = positions.as_deref_mut() {
        out.clear();
    }
    let mut score = (LATE_START * start as i32).max(LATE_START_MAX);
    let mut prev: Option<usize> = None;
    let mut from = start;
    for &unit in needle {
        let i = from + lower[from..].iter().position(|&u| u == unit)?;
        score += MATCH + boundary(orig, i) + if i >= base { BASENAME } else { 0 };
        if let Some(p) = prev {
            let gap = (i - p - 1) as i32;
            score += if gap == 0 { CONTIGUOUS } else { (GAP_OPEN + GAP_EXTEND * (gap - 1)).max(GAP_MAX) };
        }
        if let Some(out) = positions.as_deref_mut() {
            out.push(i as u32);
        }
        prev = Some(i);
        from = i + 1;
    }
    let contiguous = prev == Some(start + needle.len() - 1);
    if contiguous && start == base {
        score += PREFIX;
    }
    if contiguous && start == 0 && lower.len() == needle.len() {
        score += EXACT;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(query: &str, candidate: &str) -> i32 {
        fuzzy_match(query, candidate).map(|m| m.score).unwrap_or_else(|| panic!("{query} ~ {candidate}"))
    }

    fn pos(query: &str, candidate: &str) -> Vec<u32> {
        fuzzy_match(query, candidate).unwrap().positions
    }

    #[test]
    fn a_subsequence_matches_case_insensitively_and_anything_else_does_not() {
        assert_eq!(pos("hud", "ObjectiveHUD"), vec![9, 10, 11]);
        assert!(fuzzy_match("hdu", "hud").is_none(), "order matters");
        assert!(fuzzy_match("", "anything").is_none(), "an empty query matches nothing");
        assert!(fuzzy_match("longer", "long").is_none());
        assert_eq!(pos("objective hud", "ObjectiveHud").len(), 12, "whitespace in the query is ignored");
    }

    #[test]
    fn a_contiguous_run_beats_a_scattered_match() {
        assert!(s("abc", "xabcx") > s("abc", "xaxbxcx"));
        // Each gap costs, and a longer gap costs more, up to the cap.
        assert_eq!(s("ab", "a_xb") - s("ab", "a_xxb"), -GAP_EXTEND);
        assert_eq!(s("ab", "axxxxxxxxxxxxxxxxxxxxxxxb"), s("ab", "axxxxxxxxxxxxxxxxxxxxxxxxxxxxxb"));
    }

    #[test]
    fn a_word_start_beats_the_middle_of_a_word() {
        for sep in ["/", "_", "-", ".", ":", " "] {
            let at_word = format!("xx{sep}hud");
            assert!(s("hud", &at_word) > s("hud", "xxxhud"), "after {sep:?}");
        }
        // The best start is found even when an earlier occurrence exists.
        assert_eq!(pos("hud", "hello_hud"), vec![6, 7, 8]);
    }

    #[test]
    fn a_camel_hump_beats_a_lowercase_letter() {
        assert!(s("oh", "objectHud") > s("oh", "objecthud"));
        assert_eq!(pos("objhud", "SCR_ObjectiveHudComponent"), vec![4, 5, 6, 13, 14, 15]);
    }

    #[test]
    fn a_match_in_the_basename_beats_one_in_a_folder() {
        assert!(s("hud", "x/hud.rs") > s("hud", "hud/x.rs"));
        assert_eq!(pos("hud", "hud/hud.rs"), vec![4, 5, 6], "the basename occurrence wins");
    }

    #[test]
    fn a_prefix_and_an_exact_match_rank_first() {
        assert_eq!(s("hud", "hud.rs") - s("hud", "_hud.rs"), PREFIX - LATE_START);
        assert_eq!(s("hud", "hud") - s("hud", "hud.rs"), EXACT);
        assert_eq!(s("HUD", "hud"), s("hud", "HUD"), "case never changes the score");
    }

    #[test]
    fn a_late_first_match_costs_up_to_a_cap() {
        assert_eq!(s("ab", "_ab") - s("ab", "__ab"), -LATE_START);
        let far = format!("{}_ab", "x".repeat(40));
        let farther = format!("{}_ab", "x".repeat(80));
        assert_eq!(s("ab", &far), s("ab", &farther));
    }

    #[test]
    fn non_ascii_text_matches_by_char_and_reports_byte_offsets() {
        // "Größe" is G r ö ß e: ö and ß take two bytes each.
        assert_eq!(pos("öße", "Größe"), vec![2, 4, 6]);
        assert_eq!(pos("ÉT", "été"), vec![0, 2]);
        assert_eq!(pos("ab", "äab"), vec![2, 3], "an ASCII query against non-ASCII text");
        assert!(fuzzy_match("ö", "oe").is_none());
        assert_eq!(fold_lower("ÄÖÜ"), "äöü");
    }

    #[test]
    fn the_same_input_always_gives_the_same_match() {
        let a = fuzzy_match("sc", "src/scene/scan.rs");
        assert_eq!(a, fuzzy_match("sc", "src/scene/scan.rs"));
        assert_eq!(a.unwrap().positions, vec![10, 11], "the basename's word start");
    }
}
