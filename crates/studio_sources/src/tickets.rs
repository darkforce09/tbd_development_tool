//! Tickets kept in the repository: the `T-*.toml` files directly in `.ai/tickets/`. Each ticket's
//! `shipped_at` commit is confirmed with git (one `git cat-file --batch-check` for all of them)
//! before Studio calls it shipped in this repo. Nothing is cached: the folder parses in
//! milliseconds.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::exec::{Command, ExecError, Runner};
use crate::hub::{JobContext, JobError, SourceEvent};

/// The only folder tickets are read from, relative to the project root.
pub const TICKETS_FOLDER: &str = ".ai/tickets";

/// Every ticket of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketIndex {
    /// Sorted by id, numerically per segment ([`id_order`]).
    pub tickets: Vec<Ticket>,
    /// Tickets per `status`, as written.
    pub status_counts: BTreeMap<String, usize>,
    /// `T-*.toml` files found, read or not.
    pub files: usize,
    /// Files that could not be read: file name and why (never the file's text).
    pub bad_files: Vec<(String, String)>,
    /// The `.ai/tickets` folder.
    pub folder: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub status: String,
    /// `work` or `program`.
    pub kind: Option<String>,
    pub priority: Option<i64>,
    pub order: Option<i64>,
    pub parent: Option<String>,
    /// The commit id as written in the ticket (4–40 hex, or a full 64-hex SHA-256 id).
    pub shipped_at: Option<String>,
    /// What git says about `shipped_at`.
    pub shipped: Shipped,
    /// Repo-relative path of the spec `.md`, as written.
    pub spec: Option<String>,
    /// Repo-relative path of the plan `.md`, as written.
    pub plan: Option<String>,
    /// Repo-relative path of the ticket's `.toml`.
    pub file: String,
}

/// Whether a ticket's `shipped_at` commit is in this repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shipped {
    /// No `shipped_at`, or it was not checked: there is no git repository (or no git) here. When
    /// `shipped_at` is set the UI says "not checked".
    None,
    /// Git confirmed the commit: its full 40-hex id. A Proven link (basis: commit id).
    Commit(String),
    /// Git does not know the commit (missing, ambiguous or not a commit id): "commit not in this
    /// repo".
    NotInRepo,
}

/// Reads every `T-*.toml` directly in `<root>/.ai/tickets` and confirms their `shipped_at`
/// commits. A file that does not parse goes to `bad_files`; it never stops the read.
pub fn read_tickets(runner: &Runner, root: &Path) -> Result<TicketIndex, JobError> {
    let folder = root.join(TICKETS_FOLDER);
    let entries =
        std::fs::read_dir(&folder).map_err(|_| JobError::Unavailable("No tickets here (.ai/tickets)".to_string()))?;
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with("T-") && name.ends_with(".toml"))
        .collect();
    names.sort();
    if runner.cancel_token().is_cancelled() {
        return Err(JobError::Cancelled);
    }

    let parsed: Vec<(String, Result<Ticket, String>)> = names
        .par_iter()
        .map(|name| {
            let file = format!("{TICKETS_FOLDER}/{name}");
            (
                name.clone(),
                std::fs::read_to_string(folder.join(name))
                    .map_err(|e| e.to_string())
                    .and_then(|text| parse_ticket(&text, file)),
            )
        })
        .collect();
    let mut tickets = Vec::with_capacity(parsed.len());
    let mut bad_files = Vec::new();
    for (name, ticket) in parsed {
        match ticket {
            Ok(t) => tickets.push(t),
            Err(why) => bad_files.push((name, why)),
        }
    }
    tickets.sort_by(|a, b| id_order(&a.id, &b.id));

    let mut status_counts = BTreeMap::new();
    for t in &tickets {
        *status_counts.entry(t.status.clone()).or_insert(0) += 1;
    }
    confirm_shipped(runner, root, &mut tickets)?;
    Ok(TicketIndex { tickets, status_counts, files: names.len(), bad_files, folder })
}

/// One ticket from its TOML. `id` and `status` are required; a field of the wrong type counts as
/// absent. Errors name the place, never the text (TOML errors quote the line, so only their first
/// line is kept).
fn parse_ticket(text: &str, file: String) -> Result<Ticket, String> {
    let table: toml::Table =
        text.parse().map_err(|e: toml::de::Error| e.to_string().lines().next().unwrap_or_default().to_string())?;
    let string = |key: &str| table.get(key).and_then(toml::Value::as_str).map(str::to_string);
    let int = |key: &str| table.get(key).and_then(toml::Value::as_integer);
    let id = string("id").ok_or("no `id`")?;
    let status = string("status").ok_or("no `status`")?;
    Ok(Ticket {
        id,
        title: string("title").unwrap_or_default(),
        status,
        kind: string("kind"),
        priority: int("priority"),
        order: int("order"),
        parent: string("parent"),
        shipped_at: string("shipped_at"),
        shipped: Shipped::None,
        spec: string("spec"),
        plan: string("plan"),
        file,
    })
}

/// Fills `shipped` for every ticket with a `shipped_at`, asking one `git cat-file --batch-check`.
/// No git repository, or no git, leaves them all [`Shipped::None`] (not checked).
fn confirm_shipped(runner: &Runner, root: &Path, tickets: &mut [Ticket]) -> Result<(), JobError> {
    let asked: Vec<String> = tickets
        .iter()
        .filter_map(|t| t.shipped_at.as_deref())
        .filter(|sha| is_commit_id(sha))
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut known: BTreeMap<String, Shipped> = BTreeMap::new();
    if !asked.is_empty() {
        let input: String = asked.iter().map(|sha| format!("{sha}^{{commit}}\n")).collect();
        let command = Command::git(root, ["cat-file", "--batch-check"])?.with_stdin(input.into_bytes());
        match runner.run(&command) {
            Ok(out) => {
                let answers = parse_batch_check(&asked, &String::from_utf8_lossy(&out.stdout));
                known = asked.into_iter().zip(answers).collect();
            }
            Err(ExecError::Cancelled) => return Err(JobError::Cancelled),
            // Not a git repository, no git, or git gave up: nothing is checked.
            Err(_) => return Ok(()),
        }
    }
    for t in tickets.iter_mut() {
        let Some(sha) = t.shipped_at.as_deref() else { continue };
        t.shipped = if is_commit_id(sha) {
            known.get(&sha.to_ascii_lowercase()).cloned().unwrap_or(Shipped::None)
        } else {
            Shipped::NotInRepo
        };
    }
    Ok(())
}

/// 4 to 40 hex digits (a SHA-1 id or a prefix), or a full 64-digit SHA-256 id: something git
/// can look up as a commit.
fn is_commit_id(sha: &str) -> bool {
    ((4..=40).contains(&sha.len()) || sha.len() == 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Git answers each line in order: `<full id> commit <size>` when found, `<input> missing` or
/// `<input> ambiguous` otherwise. A found id must extend the one asked.
fn parse_batch_check(asked: &[String], stdout: &str) -> Vec<Shipped> {
    let mut lines = stdout.lines();
    asked
        .iter()
        .map(|sha| {
            let fields: Vec<&str> = lines.next().unwrap_or_default().split_whitespace().collect();
            match fields[..] {
                [full, "commit", _] if matches!(full.len(), 40 | 64) && full.starts_with(sha.as_str()) => {
                    Shipped::Commit(full.to_string())
                }
                _ => Shipped::NotInRepo,
            }
        })
        .collect()
}

/// The job for a [`crate::SourceHub`]; sends [`SourceEvent::Tickets`].
pub fn tickets_job() -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send {
    move |ctx| {
        let index = read_tickets(&ctx.runner, &ctx.root)?;
        ctx.send(SourceEvent::Tickets(std::sync::Arc::new(index)));
        Ok(())
    }
}

/// The exact ticket ids (`T-<n>(.<n>)*`) in a branch name, in order, each once. A token starts at
/// the start or after a character that is not a letter or digit, and ends at the end or before a
/// character that is not a letter or digit and is not a `.` or `-` followed by a digit:
/// `slice/T-940.11` → `T-940.11`, `T-1-fix` → `T-1`, `XT-9` and `T-1-2` → nothing.
///
/// This is name matching: a link it makes is Unresolved (basis name match, "branch name") and is
/// drawn dashed and muted.
pub fn ticket_ids_in(name: &str) -> Vec<String> {
    let bytes = name.as_bytes();
    let digits_from = |mut i: usize| {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    let mut ids: Vec<String> = Vec::new();
    let mut start = 0;
    while let Some(found) = name[start..].find("T-") {
        let at = start + found;
        start = at + 1;
        if at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
            continue;
        }
        let mut end = digits_from(at + 2);
        if end == at + 2 {
            continue;
        }
        while end + 1 < bytes.len() && bytes[end] == b'.' && bytes[end + 1].is_ascii_digit() {
            end = digits_from(end + 1);
        }
        let continues = match bytes.get(end) {
            None => false,
            Some(c) if c.is_ascii_alphanumeric() => true,
            Some(b'.' | b'-') => bytes.get(end + 1).is_some_and(u8::is_ascii_digit),
            Some(_) => false,
        };
        if continues {
            continue;
        }
        let id = &name[at..end];
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_string());
        }
        start = end;
    }
    ids
}

/// Orders ticket ids numerically per segment: `T-9` < `T-10`, `T-940` < `T-940.2` < `T-940.11`.
/// Segments that are not numbers sort after numbers, as text.
pub fn id_order(a: &str, b: &str) -> Ordering {
    let segments = |id: &str| -> Vec<Result<u64, String>> {
        id.strip_prefix("T-").unwrap_or(id).split('.').map(|s| s.parse::<u64>().map_err(|_| s.to_string())).collect()
    };
    let key = |s: &Result<u64, String>| match s {
        Ok(n) => (0, *n, String::new()),
        Err(text) => (1, 0, text.clone()),
    };
    let (sa, sb) = (segments(a), segments(b));
    sa.iter().map(key).cmp(sb.iter().map(key)).then_with(|| a.cmp(b))
}

/// What to work on next: `ready` tickets, then `queued` ones, each by priority (lowest first, none
/// last), then `order` (none last), then id. At most `n`.
pub fn next_tickets(index: &TicketIndex, n: usize) -> Vec<&Ticket> {
    let rank = |status: &str| match status {
        "ready" => Some(0),
        "queued" => Some(1),
        _ => None,
    };
    let mut next: Vec<(usize, &Ticket)> = index.tickets.iter().filter_map(|t| Some((rank(&t.status)?, t))).collect();
    let last = |v: Option<i64>| (v.is_none(), v.unwrap_or(0));
    next.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then(last(a.priority).cmp(&last(b.priority)))
            .then(last(a.order).cmp(&last(b.order)))
            .then_with(|| id_order(&a.id, &b.id))
    });
    next.into_iter().take(n).map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_ids_are_exact_tokens_in_a_branch_name() {
        let ids = |name: &str| ticket_ids_in(name);
        assert_eq!(ids("slice/T-940.11"), ["T-940.11"]);
        assert_eq!(ids("T-1-fix"), ["T-1"]);
        assert_eq!(ids("XT-9"), Vec::<String>::new());
        assert_eq!(ids("T-1-2"), Vec::<String>::new());
        assert_eq!(ids("T-12abc"), Vec::<String>::new());
        assert_eq!(ids("T-"), Vec::<String>::new());
        assert_eq!(ids("T-x"), Vec::<String>::new());
        assert_eq!(ids("T-1.2.3.4"), ["T-1.2.3.4"]);
        assert_eq!(ids("wip/T-7.x"), ["T-7"]);
        assert_eq!(ids("T-7."), ["T-7"]);
        assert_eq!(ids("feat_T-9_and_T-10"), ["T-9", "T-10"]);
        assert_eq!(ids("T-3-T-4/T-3"), ["T-3", "T-4"]);
        assert_eq!(ids("t-5"), Vec::<String>::new());
        assert_eq!(ids("main"), Vec::<String>::new());
    }

    #[test]
    fn ids_order_numerically_per_segment() {
        let mut ids = vec!["T-940.11", "T-10", "T-940", "T-9", "T-940.2", "T-1.2.3.4", "T-1", "T-1.2", "T-x"];
        ids.sort_by(|a, b| id_order(a, b));
        assert_eq!(ids, ["T-1", "T-1.2", "T-1.2.3.4", "T-9", "T-10", "T-940", "T-940.2", "T-940.11", "T-x"]);
        assert_eq!(id_order("T-940.2", "T-940.2"), Ordering::Equal);
    }

    #[test]
    fn batch_check_answers_map_to_what_was_asked() {
        let full = "0123456789abcdef0123456789abcdef01234567";
        let asked = ["0123456".to_string(), "abcd".to_string(), "deadbeef".to_string(), "ffff".to_string()];
        let out = format!("{full} commit 230\nabcd^{{commit}} ambiguous\ndeadbeef^{{commit}} missing\n");
        assert_eq!(
            parse_batch_check(&asked, &out),
            [Shipped::Commit(full.to_string()), Shipped::NotInRepo, Shipped::NotInRepo, Shipped::NotInRepo]
        );
        // A found id that does not extend the one asked is not trusted.
        assert_eq!(parse_batch_check(&["fff0".to_string()], &format!("{full} commit 1\n")), [Shipped::NotInRepo]);
        assert!(is_commit_id("abc1234") && !is_commit_id("abc") && !is_commit_id("v1.2.3") && !is_commit_id(""));
        // A SHA-256 repository answers with a 64-digit id.
        let full256 = "0123456789abcdef".repeat(4);
        assert_eq!(
            parse_batch_check(&["01234567".to_string()], &format!("{full256} commit 9\n")),
            [Shipped::Commit(full256.clone())]
        );
    }

    #[test]
    fn commit_ids_may_be_sha1_or_sha256() {
        assert!(is_commit_id(&"a".repeat(40)), "SHA-1");
        assert!(is_commit_id(&"b".repeat(64)), "SHA-256");
        assert!(!is_commit_id(&"c".repeat(41)) && !is_commit_id(&"d".repeat(63)) && !is_commit_id(&"e".repeat(65)));
        assert!(!is_commit_id(&format!("{}g", "a".repeat(63))), "hex only");
    }

    #[test]
    fn a_ticket_needs_an_id_and_a_status_and_bad_toml_names_no_text() {
        let file = || "x.toml".to_string();
        let t = parse_ticket("id = \"T-1\"\nstatus = \"idea\"\npriority = \"high\"\n", file()).unwrap();
        assert_eq!((t.id.as_str(), t.priority, t.title.as_str()), ("T-1", None, ""));
        assert_eq!(parse_ticket("status = \"idea\"", file()), Err("no `id`".to_string()));
        assert_eq!(parse_ticket("id = \"T-1\"", file()), Err("no `status`".to_string()));
        let err = parse_ticket("id = \"T-1\"\nsecret words here = = \n", file()).unwrap_err();
        assert!(!err.contains("secret") && !err.contains('\n'), "{err}");
    }
}
