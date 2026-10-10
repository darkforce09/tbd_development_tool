//! `@route` and `@contract` tags written in source comments.
//!
//! A tag counts only inside a comment of a code file (Rust, Enforce and the tree-sitter languages); Markdown, data
//! and config files never carry tags. The tag must be the first word of its comment line:
//!
//! - `@route <METHODS> <TEMPLATES> [anything]` where METHODS is one or more `|`-separated words (upper-cased) and
//!   TEMPLATES is one or more `|`-separated templates, each starting with `/` or being a full URL;
//! - `@contract <file>[#<pointer>] [partial] [anything]` (`partial` counts only as the word right after the file).
//!
//! A tag that does not follow its syntax is kept in [`FileTags::malformed`] and never produces a link.
//!
//! # Comment syntax per language
//!
//! | Languages | Line comments | Block comments | Strings the scanner skips |
//! |---|---|---|---|
//! | Rust | `//` `///` `//!` | `/* */` nested | `"…"` (multi-line), `r#"…"#`, `'c'` (lifetimes are code) |
//! | Enforce | `//` `//!` | `/* */` | `"…"`, `'…'` |
//! | C, C++ | `//` | `/* */` | `"…"`, `'…'` (not a digit separator `1'000`), C++ `R"d(…)d"` |
//! | C# | `//` | `/* */` | `"…"`, `'…'`, `@"…"`, `"""…"""` (raw: no escapes) |
//! | Java, Go, JavaScript, TypeScript, TSX | `//` | `/* */` | `"…"`, `'…'`, Java `"""…"""`, backtick strings (multi-line) |
//! | Kotlin, Scala, Dart | `//` | `/* */` nested | `"…"`, `'…'` (Scala: as Rust), `"""…"""` (Kotlin, Scala: raw, no escapes; Dart also `'''`) |
//! | Swift | `//` | `/* */` nested | `"…"`, `"""…"""` |
//! | Zig | `//` | none | `"…"`, `'…'`, `\\` multi-line string lines |
//! | PHP | `//`, `#` (not `#[`) | `/* */` | `"…"`, `'…'` (multi-line) |
//! | Python, Ruby | `#` | none | `"…"`, `'…'`, `"""…"""`, `'''…'''` (Python docstrings are strings: no tags) |
//! | Bash | `#` at the start of a word | none | `"…"`, `'…'` (no escapes; both multi-line) |
//! | Lua | `--` | `--[[ ]]`, `--[==[ ]==]` | `"…"`, `'…'`, `[[…]]`, `[==[…]==]` |
//!
//! None of these languages uses `;` comments, and SQL is not a code language here. Not handled (a tag-like text
//! inside these is read as written): JavaScript regex literals, heredocs (Bash, PHP, Ruby), Swift `#"…"#` raw
//! strings, Ruby `=begin`/`=end` blocks and `%q()` strings. Backslash escapes are honoured in every quoted string
//! except Bash single quotes, C# verbatim and raw strings, Kotlin and Scala triple-quoted strings and Rust/C++ raw
//! strings.
//!
//! # Owner
//!
//! The comment block holding a tag is the run of comment-only lines from the tag down; a blank line or a code line
//! ends it. Attribute and decorator lines directly below the block are stepped over (`#[…]` in Rust and PHP,
//! `@…` in languages with annotations or decorators, `[…]` in C# and Enforce). The tag belongs to the item whose
//! span starts on one of those lines or on the first line after them; otherwise it belongs to the file. A tag in a
//! comment that shares its line with code belongs to the file. Rust inner doc comments (`//!`, `/*!`) document the
//! item they sit in: the innermost span that contains them, else the file.

use std::path::{Path, PathBuf};

use crate::extractor::lang::SourceLang;
use crate::extractor::treesitter::CodeLang;
use crate::extractor::types::ExtractedFile;

/// An item a tag can belong to: its name and 1-based first and last line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnerSpan {
    pub name: String,
    pub line: usize,
    pub line_end: usize,
}

/// What a tag documents.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagOwner {
    Item(OwnerSpan),
    File,
}

/// `@route GET|POST /a|/b`: one tag, every method and template as written (methods upper-cased).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTag {
    pub methods: Vec<String>,
    pub templates: Vec<String>,
    pub file: PathBuf,
    pub line: usize,
    pub owner: TagOwner,
}

/// `@contract things.schema.json#/definitions/Done partial`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractTag {
    pub file_ref: String,
    /// The JSON pointer after `#`, as written; `None` when there is no `#` or nothing after it.
    pub pointer: Option<String>,
    pub partial: bool,
    pub file: PathBuf,
    pub line: usize,
    pub owner: TagOwner,
}

/// Every tag of one file, in line order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTags {
    pub routes: Vec<RouteTag>,
    pub contracts: Vec<ContractTag>,
    /// Tags that do not follow their syntax: (line, reason).
    pub malformed: Vec<(usize, String)>,
}

/// Finds the tags in `text`. `owners` are the item spans the extractor found in the file, in any order.
pub fn scan_tags(file: &Path, text: &str, lang: SourceLang, owners: &[OwnerSpan]) -> FileTags {
    let mut out = FileTags::default();
    let Some(syntax) = Syntax::of(lang) else { return out };
    let lines = classify(text, &syntax);
    let line_texts: Vec<&str> = text.split('\n').collect();

    for (idx, info) in lines.iter().enumerate() {
        for piece in &info.comments {
            let body = piece.body(text);
            let (is_route, rest) = if let Some(rest) = tag_rest(body, "@route") {
                (true, rest)
            } else if let Some(rest) = tag_rest(body, "@contract") {
                (false, rest)
            } else {
                continue;
            };
            let line = idx + 1;
            let owner = if piece.inner_doc {
                enclosing_owner(line, owners)
            } else {
                block_owner(idx, &lines, &line_texts, &syntax, owners)
            };
            let parsed = if is_route { parse_route(rest) } else { parse_contract(rest) };
            match parsed {
                Ok(Parsed::Route { methods, templates }) => {
                    out.routes.push(RouteTag { methods, templates, file: file.to_path_buf(), line, owner })
                }
                Ok(Parsed::Contract { file_ref, pointer, partial }) => out.contracts.push(ContractTag {
                    file_ref,
                    pointer,
                    partial,
                    file: file.to_path_buf(),
                    line,
                    owner,
                }),
                Err(reason) => out.malformed.push((line, reason)),
            }
        }
    }
    out
}

/// The spans of the items an extractor found: functions, methods, structs/classes, enums and traits, sorted.
/// Rust `impl` blocks carry no end line and are left out (their methods are in).
pub fn owner_spans(file: &ExtractedFile) -> Vec<OwnerSpan> {
    let span = |name: &str, line: usize, line_end: usize| OwnerSpan { name: name.to_string(), line, line_end };
    let mut spans: Vec<OwnerSpan> = file
        .functions
        .iter()
        .chain(file.impls.iter().flat_map(|i| &i.methods))
        .map(|f| span(&f.name, f.line, f.line_end))
        .chain(file.structs.iter().map(|s| span(&s.name, s.line, s.line_end)))
        .chain(file.enums.iter().map(|e| span(&e.name, e.line, e.line_end)))
        .chain(file.traits.iter().map(|t| span(&t.name, t.line, t.line_end)))
        .collect();
    spans.sort_by(|a, b| (a.line, a.line_end, &a.name).cmp(&(b.line, b.line_end, &b.name)));
    spans.dedup();
    spans
}

/// The text after `tag` when the comment line starts with it as a whole word.
fn tag_rest<'a>(body: &'a str, tag: &str) -> Option<&'a str> {
    let rest = body.trim_start().strip_prefix(tag)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest)
}

enum Parsed {
    Route { methods: Vec<String>, templates: Vec<String> },
    Contract { file_ref: String, pointer: Option<String>, partial: bool },
}

fn parse_route(rest: &str) -> Result<Parsed, String> {
    let mut words = rest.split_whitespace();
    let Some(methods_text) = words.next() else { return Err("@route without method and template".to_string()) };
    if is_template(methods_text.split('|').next().unwrap_or("")) {
        return Err(format!("@route without HTTP method before {methods_text}"));
    }
    let methods: Vec<&str> = methods_text.split('|').collect();
    if methods.iter().any(|m| m.is_empty() || !m.chars().all(|c| c.is_ascii_alphabetic())) {
        return Err(format!("@route method list {methods_text} is not words separated by |"));
    }
    let Some(templates_text) = words.next() else { return Err(format!("@route {methods_text} without template")) };
    let templates: Vec<&str> = templates_text.split('|').collect();
    if let Some(bad) = templates.iter().find(|t| !is_template(t)) {
        return Err(format!("@route template {bad:?} does not start with / and is not a URL"));
    }
    Ok(Parsed::Route {
        methods: methods.iter().map(|m| m.to_ascii_uppercase()).collect(),
        templates: templates.iter().map(|t| t.to_string()).collect(),
    })
}

/// Starts with `/`, or is `scheme://…` with a letters-only scheme.
fn is_template(text: &str) -> bool {
    text.starts_with('/')
        || text.split_once("://").is_some_and(|(scheme, _)| {
            !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        })
}

fn parse_contract(rest: &str) -> Result<Parsed, String> {
    let mut words = rest.split_whitespace();
    let Some(target) = words.next() else { return Err("@contract without file".to_string()) };
    let (file_ref, pointer) = match target.split_once('#') {
        Some((file, pointer)) => (file, (!pointer.is_empty()).then(|| pointer.to_string())),
        None => (target, None),
    };
    if file_ref.is_empty() {
        return Err(format!("@contract {target} has no file name before #"));
    }
    let partial = words.next() == Some("partial");
    Ok(Parsed::Contract { file_ref: file_ref.to_string(), pointer, partial })
}

/// Rust inner doc: the innermost span strictly containing the line.
fn enclosing_owner(line: usize, owners: &[OwnerSpan]) -> TagOwner {
    owners
        .iter()
        .filter(|o| o.line < line && line <= o.line_end)
        .min_by(|a, b| (a.line_end - a.line, &a.name, a.line).cmp(&(b.line_end - b.line, &b.name, b.line)))
        .map_or(TagOwner::File, |o| TagOwner::Item(o.clone()))
}

/// The item that starts right after the comment block holding line `idx` (0-based).
fn block_owner(idx: usize, lines: &[LineInfo], texts: &[&str], syntax: &Syntax, owners: &[OwnerSpan]) -> TagOwner {
    if lines[idx].code {
        return TagOwner::File;
    }
    let mut end = idx;
    while end + 1 < lines.len() && lines[end + 1].is_comment_only() {
        end += 1;
    }
    let first = end + 1; // 0-based first line after the block
    let mut item = first;
    while item < lines.len() && lines[item].code && syntax.is_attribute(texts[item]) {
        // A multi-line attribute runs until its brackets close.
        let mut depth = 0i32;
        loop {
            depth += bracket_balance(texts[item]);
            item += 1;
            if depth <= 0 || item >= lines.len() || !lines[item].code {
                break;
            }
        }
    }
    let (from, to) = (first + 1, item + 1); // 1-based line range that may hold the owner's first line
    owners
        .iter()
        .filter(|o| from <= o.line && o.line <= to)
        .min_by(|a, b| (a.line, b.line_end, &a.name).cmp(&(b.line, a.line_end, &b.name)))
        .map_or(TagOwner::File, |o| TagOwner::Item(o.clone()))
}

fn bracket_balance(text: &str) -> i32 {
    text.chars()
        .map(|c| match c {
            '(' | '[' | '{' => 1,
            ')' | ']' | '}' => -1,
            _ => 0,
        })
        .sum()
}

// ----- comment scanner -----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hash {
    None,
    Anywhere,
    /// Bash: `#` starts a comment only at the start of a word.
    WordStart,
    /// PHP: `#[` is an attribute.
    NotAttribute,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Single {
    /// `'…'` is a string or char literal with escapes.
    Str,
    /// Bash: `'…'` without escapes, may span lines.
    Raw,
    /// Rust and Scala: a char literal only when it closes after one char or escape; otherwise a lifetime or symbol.
    CharOrCode,
    /// Swift: `'` is not a literal.
    Code,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backtick {
    None,
    /// JavaScript template literal: escapes, multi-line.
    Template,
    /// Go raw string: no escapes, multi-line.
    Raw,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attr {
    None,
    /// `#[…]`
    HashBracket,
    /// `@name`
    At,
    /// `[…]`
    Bracket,
}

#[derive(Clone, Copy)]
struct Syntax {
    slash_line: bool,
    block: bool,
    nested_blocks: bool,
    hash: Hash,
    dash: bool,
    single: Single,
    backtick: Backtick,
    /// `"""` (and `'''` when single quotes are strings) open multi-line strings.
    triple: bool,
    /// Backslash escapes count inside triple-quoted strings (not in Kotlin, Scala and C# raw strings).
    triple_escapes: bool,
    /// `'` between digits of a number is a digit separator (C23, C++14), not a character literal.
    digit_separators: bool,
    /// `"…"` may span lines without a trailing backslash.
    multiline_quotes: bool,
    rust_raw: bool,
    cpp_raw: bool,
    cs_verbatim: bool,
    zig_lines: bool,
    lua_long: bool,
    /// `//!` and `/*!` are inner doc comments (Rust).
    inner_docs: bool,
    attr: Attr,
}

impl Syntax {
    fn of(lang: SourceLang) -> Option<Syntax> {
        let c_like = Syntax {
            slash_line: true,
            block: true,
            nested_blocks: false,
            hash: Hash::None,
            dash: false,
            single: Single::Str,
            backtick: Backtick::None,
            triple: false,
            triple_escapes: true,
            digit_separators: false,
            multiline_quotes: false,
            rust_raw: false,
            cpp_raw: false,
            cs_verbatim: false,
            zig_lines: false,
            lua_long: false,
            inner_docs: false,
            attr: Attr::None,
        };
        let hash_like = Syntax { slash_line: false, block: false, hash: Hash::Anywhere, ..c_like };
        let code = match lang {
            SourceLang::Rust => Syntax {
                nested_blocks: true,
                single: Single::CharOrCode,
                multiline_quotes: true,
                rust_raw: true,
                inner_docs: true,
                attr: Attr::HashBracket,
                ..c_like
            },
            SourceLang::Enforce => Syntax { attr: Attr::Bracket, ..c_like },
            SourceLang::Code(code) => match code {
                CodeLang::C => Syntax { digit_separators: true, ..c_like },
                CodeLang::Cpp => Syntax { cpp_raw: true, digit_separators: true, ..c_like },
                CodeLang::CSharp => {
                    Syntax { cs_verbatim: true, triple: true, triple_escapes: false, attr: Attr::Bracket, ..c_like }
                }
                CodeLang::Java => Syntax { triple: true, attr: Attr::At, ..c_like },
                CodeLang::Go => Syntax { backtick: Backtick::Raw, ..c_like },
                CodeLang::JavaScript | CodeLang::TypeScript | CodeLang::Tsx => {
                    Syntax { backtick: Backtick::Template, attr: Attr::At, ..c_like }
                }
                CodeLang::Kotlin => {
                    Syntax { nested_blocks: true, triple: true, triple_escapes: false, attr: Attr::At, ..c_like }
                }
                CodeLang::Dart => Syntax { nested_blocks: true, triple: true, attr: Attr::At, ..c_like },
                CodeLang::Scala => Syntax {
                    nested_blocks: true,
                    triple: true,
                    triple_escapes: false,
                    single: Single::CharOrCode,
                    attr: Attr::At,
                    ..c_like
                },
                CodeLang::Swift => {
                    Syntax { nested_blocks: true, triple: true, single: Single::Code, attr: Attr::At, ..c_like }
                }
                CodeLang::Zig => Syntax { block: false, zig_lines: true, ..c_like },
                CodeLang::Php => {
                    Syntax { hash: Hash::NotAttribute, multiline_quotes: true, attr: Attr::HashBracket, ..c_like }
                }
                CodeLang::Python => Syntax { triple: true, attr: Attr::At, ..hash_like },
                CodeLang::Ruby => Syntax { multiline_quotes: true, ..hash_like },
                CodeLang::Bash => {
                    Syntax { hash: Hash::WordStart, single: Single::Raw, multiline_quotes: true, ..hash_like }
                }
                CodeLang::Lua => Syntax { hash: Hash::None, dash: true, lua_long: true, ..hash_like },
            },
            SourceLang::Markdown | SourceLang::Other => return None,
        };
        Some(code)
    }

    fn is_attribute(&self, line: &str) -> bool {
        let t = line.trim_start();
        match self.attr {
            Attr::None => false,
            Attr::HashBracket => t.starts_with("#["),
            Attr::At => t.strip_prefix('@').and_then(|r| r.chars().next()).is_some_and(|c| c.is_alphabetic()),
            Attr::Bracket => t.starts_with('[') && !t.starts_with("[["),
        }
    }
}

/// A piece of comment on one line: a byte range of the text.
#[derive(Debug, Clone, Copy)]
struct Piece {
    start: usize,
    end: usize,
    /// Continuation line of a block comment: a leading `*` is decoration.
    continuation: bool,
    inner_doc: bool,
}

impl Piece {
    /// The comment text without its markers: `///`, `//!`, `#`, `--`, `/**` or a leading `*` are gone.
    fn body<'a>(&self, text: &'a str) -> &'a str {
        let raw = &text[self.start..self.end];
        if self.continuation {
            let t = raw.trim_start();
            t.strip_prefix('*').map_or(t, |r| r.trim_start_matches('*'))
        } else {
            let t = raw.trim_start_matches(['/', '*', '#', '-']);
            t.strip_prefix('!').unwrap_or(t)
        }
    }
}

#[derive(Debug, Default)]
struct LineInfo {
    code: bool,
    comments: Vec<Piece>,
}

impl LineInfo {
    fn is_comment_only(&self) -> bool {
        !self.code && !self.comments.is_empty()
    }
}

#[derive(Clone, Copy)]
enum Close {
    /// A quote char; `escapes` honours backslashes; `multiline` lets a bare newline continue it.
    Quote {
        quote: u8,
        escapes: bool,
        multiline: bool,
    },
    /// A triple quote char; `escapes` honours backslashes.
    Triple {
        quote: u8,
        escapes: bool,
    },
    RustRaw(usize),
    CsVerbatim,
    LuaLong(usize),
    /// Zig `\\` string line, or an unterminated literal: ends at the newline.
    EndOfLine,
}

enum State {
    Code,
    LineComment { start: usize, inner_doc: bool },
    Block { start: usize, depth: usize, lua: Option<usize>, continuation: bool, inner_doc: bool },
    Str(Close),
    CppRaw(Vec<u8>),
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The `'` at `i` sits inside a number literal (`1'000`, `0x1'FF`): the word before it starts with a digit and a
/// digit-like char follows. A prefixed char literal (`L'x'`, `u8'x'`) starts with a letter.
fn is_digit_separator(b: &[u8], i: usize) -> bool {
    let mut start = i;
    while start > 0 && (is_ident(b[start - 1]) || b[start - 1] == b'\'') {
        start -= 1;
    }
    start < i && b[start].is_ascii_digit() && b.get(i + 1).is_some_and(|&n| n.is_ascii_alphanumeric())
}

/// `[`, `=`*n, `[` at `i` → n.
fn lua_long_open(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let level = b[i + 1..].iter().take_while(|&&c| c == b'=').count();
    (b.get(i + 1 + level) == Some(&b'[')).then_some(level)
}

fn lua_long_close(b: &[u8], i: usize, level: usize) -> bool {
    b.get(i) == Some(&b']')
        && b[i + 1..].iter().take(level).filter(|&&c| c == b'=').count() == level
        && b.get(i + 1 + level) == Some(&b']')
}

/// Splits `text` into lines that know whether they hold code and which comment pieces they hold.
fn classify(text: &str, syn: &Syntax) -> Vec<LineInfo> {
    let b = text.as_bytes();
    let mut lines = vec![LineInfo::default()];
    let mut state = State::Code;
    let mut line_start = 0usize;
    let mut i = 0usize;
    let starts = |i: usize, s: &[u8]| b[i..].starts_with(s);

    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            let cur = lines.last_mut().expect("one line at least");
            match &mut state {
                State::LineComment { start, inner_doc } => {
                    cur.comments.push(Piece { start: *start, end: i, continuation: false, inner_doc: *inner_doc });
                    state = State::Code;
                }
                State::Block { start, continuation, inner_doc, .. } => {
                    cur.comments.push(Piece {
                        start: *start,
                        end: i,
                        continuation: *continuation,
                        inner_doc: *inner_doc,
                    });
                    *start = i + 1;
                    *continuation = true;
                }
                State::Str(Close::EndOfLine) | State::Str(Close::Quote { multiline: false, .. }) => {
                    state = State::Code;
                }
                _ => {}
            }
            i += 1;
            line_start = i;
            let in_string = matches!(state, State::Str(_) | State::CppRaw(_));
            lines.push(LineInfo { code: in_string, comments: Vec::new() });
            continue;
        }

        match &mut state {
            State::Code => {
                let cur = lines.last_mut().expect("one line at least");
                if c.is_ascii_whitespace() {
                    i += 1;
                } else if syn.slash_line && starts(i, b"//") {
                    let inner_doc = syn.inner_docs && b.get(i + 2) == Some(&b'!');
                    state = State::LineComment { start: i, inner_doc };
                    i += 2;
                } else if syn.block && starts(i, b"/*") {
                    let inner_doc = syn.inner_docs && b.get(i + 2) == Some(&b'!');
                    state = State::Block { start: i, depth: 1, lua: None, continuation: false, inner_doc };
                    i += 2;
                } else if c == b'#'
                    && match syn.hash {
                        Hash::None => false,
                        Hash::Anywhere => true,
                        Hash::NotAttribute => b.get(i + 1) != Some(&b'['),
                        Hash::WordStart => {
                            i == line_start || b[i - 1].is_ascii_whitespace() || b";&|()".contains(&b[i - 1])
                        }
                    }
                {
                    state = State::LineComment { start: i, inner_doc: false };
                    i += 1;
                } else if syn.dash && starts(i, b"--") {
                    if let Some(level) = lua_long_open(b, i + 2) {
                        state = State::Block {
                            start: i + 2 + level + 2,
                            depth: 1,
                            lua: Some(level),
                            continuation: false,
                            inner_doc: false,
                        };
                        i += 2 + level + 2;
                    } else {
                        state = State::LineComment { start: i, inner_doc: false };
                        i += 2;
                    }
                } else {
                    cur.code = true;
                    let prev_ident = i > 0 && is_ident(b[i - 1]);
                    if syn.triple && (starts(i, b"\"\"\"") || (syn.single == Single::Str && starts(i, b"'''"))) {
                        state = State::Str(Close::Triple { quote: c, escapes: syn.triple_escapes });
                        i += 3;
                    } else if c == b'"' {
                        state =
                            State::Str(Close::Quote { quote: b'"', escapes: true, multiline: syn.multiline_quotes });
                        i += 1;
                    } else if c == b'\'' && syn.digit_separators && is_digit_separator(b, i) {
                        i += 1;
                    } else if c == b'\'' {
                        i += 1;
                        match syn.single {
                            Single::Str => {
                                state = State::Str(Close::Quote {
                                    quote: b'\'',
                                    escapes: true,
                                    multiline: syn.multiline_quotes,
                                })
                            }
                            Single::Raw => {
                                state = State::Str(Close::Quote { quote: b'\'', escapes: false, multiline: true })
                            }
                            Single::CharOrCode => {
                                if b.get(i) == Some(&b'\\') {
                                    state = State::Str(Close::Quote { quote: b'\'', escapes: true, multiline: false });
                                } else if let Some(ch) = text.get(i..).and_then(|r| r.chars().next()) {
                                    if b.get(i + ch.len_utf8()) == Some(&b'\'') {
                                        i += ch.len_utf8() + 1;
                                    }
                                }
                            }
                            Single::Code => {}
                        }
                    } else if c == b'`' && syn.backtick != Backtick::None {
                        let escapes = syn.backtick == Backtick::Template;
                        state = State::Str(Close::Quote { quote: b'`', escapes, multiline: true });
                        i += 1;
                    } else if syn.rust_raw && !prev_ident && (c == b'r' || starts(i, b"br")) {
                        let r = if c == b'b' { i + 1 } else { i };
                        let hashes = b[r + 1..].iter().take_while(|&&h| h == b'#').count();
                        if b.get(r + 1 + hashes) == Some(&b'"') {
                            state = State::Str(Close::RustRaw(hashes));
                            i = r + 2 + hashes;
                        } else {
                            i += 1;
                        }
                    } else if syn.cpp_raw && c == b'R' && b.get(i + 1) == Some(&b'"') {
                        let open = b[i + 2..].iter().position(|&d| d == b'(' || d == b'\n' || d == b'"');
                        match open {
                            Some(p) if b[i + 2 + p] == b'(' => {
                                let mut close = vec![b')'];
                                close.extend_from_slice(&b[i + 2..i + 2 + p]);
                                close.push(b'"');
                                state = State::CppRaw(close);
                                i += 3 + p;
                            }
                            _ => i += 1,
                        }
                    } else if syn.cs_verbatim && (starts(i, b"@\"") || starts(i, b"@$\"")) {
                        state = State::Str(Close::CsVerbatim);
                        i += if b[i + 1] == b'$' { 3 } else { 2 };
                    } else if syn.zig_lines && starts(i, b"\\\\") {
                        state = State::Str(Close::EndOfLine);
                        i += 2;
                    } else if let Some(level) = lua_long_open(b, i).filter(|_| syn.lua_long) {
                        state = State::Str(Close::LuaLong(level));
                        i += level + 2;
                    } else {
                        i += 1;
                    }
                }
            }
            State::LineComment { .. } => i += 1,
            State::Block { start, depth, lua, continuation, inner_doc } => {
                let closing = match lua {
                    Some(level) => lua_long_close(b, i, *level).then_some(*level + 2),
                    None => starts(i, b"*/").then_some(2),
                };
                if let Some(len) = closing {
                    *depth -= 1;
                    if *depth == 0 {
                        let cur = lines.last_mut().expect("one line at least");
                        cur.comments.push(Piece {
                            start: *start,
                            end: i,
                            continuation: *continuation,
                            inner_doc: *inner_doc,
                        });
                        state = State::Code;
                    }
                    i += len;
                } else if lua.is_none() && syn.nested_blocks && starts(i, b"/*") {
                    *depth += 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::Str(close) => match *close {
                Close::Quote { quote, escapes, .. } => {
                    if escapes && c == b'\\' {
                        // An escaped newline continues even a single-line string.
                        if b.get(i + 1) == Some(&b'\n') {
                            lines.push(LineInfo { code: true, comments: Vec::new() });
                            line_start = i + 2;
                        }
                        i += 2;
                    } else {
                        if c == quote {
                            state = State::Code;
                        }
                        i += 1;
                    }
                }
                Close::Triple { quote, escapes } => {
                    if escapes && c == b'\\' {
                        i += if b.get(i + 1).is_some_and(|&n| n != b'\n') { 2 } else { 1 };
                    } else if b[i..].starts_with(&[quote, quote, quote]) {
                        state = State::Code;
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                Close::RustRaw(hashes) => {
                    if c == b'"' && b[i + 1..].iter().take(hashes).filter(|&&h| h == b'#').count() == hashes {
                        state = State::Code;
                        i += 1 + hashes;
                    } else {
                        i += 1;
                    }
                }
                Close::CsVerbatim => {
                    if starts(i, b"\"\"") {
                        i += 2;
                    } else {
                        if c == b'"' {
                            state = State::Code;
                        }
                        i += 1;
                    }
                }
                Close::LuaLong(level) => {
                    if lua_long_close(b, i, level) {
                        state = State::Code;
                        i += level + 2;
                    } else {
                        i += 1;
                    }
                }
                Close::EndOfLine => i += 1,
            },
            State::CppRaw(close) => {
                if b[i..].starts_with(close) {
                    i += close.len();
                    state = State::Code;
                } else {
                    i += 1;
                }
            }
        }
    }

    // The text may end inside a comment.
    let cur = lines.last_mut().expect("one line at least");
    match state {
        State::LineComment { start, inner_doc } => {
            cur.comments.push(Piece { start, end: b.len(), continuation: false, inner_doc })
        }
        State::Block { start, continuation, inner_doc, .. } => {
            cur.comments.push(Piece { start, end: b.len(), continuation, inner_doc })
        }
        _ => {}
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(name: &str, text: &str, lang: SourceLang, owners: &[OwnerSpan]) -> FileTags {
        scan_tags(Path::new(name), text, lang, owners)
    }

    fn span(name: &str, line: usize, line_end: usize) -> OwnerSpan {
        OwnerSpan { name: name.to_string(), line, line_end }
    }

    #[test]
    fn string_literals_hide_tag_text() {
        let text = "fn f() {\n    let s = \"// @route GET /x\";\n    let r = r#\"/* @route GET /y */\"#;\n}\n";
        let tags = scan("a.rs", text, SourceLang::Rust, &[]);
        assert!(tags.routes.is_empty() && tags.malformed.is_empty(), "{tags:?}");
    }

    #[test]
    fn lifetimes_do_not_open_char_literals() {
        let text = "fn f<'a>(x: &'a str) {} // @route GET /a\n";
        let tags = scan("a.rs", text, SourceLang::Rust, &[]);
        assert_eq!(tags.routes.len(), 1);
        assert_eq!(tags.routes[0].owner, TagOwner::File, "a trailing comment belongs to the file");
        let text = "let c = '\"'; // @route GET /b\n";
        assert_eq!(scan("a.rs", text, SourceLang::Rust, &[]).routes.len(), 1);
    }

    #[test]
    fn route_syntax() {
        let text = "// @route get|Post /a|https://h.example/b trailing words\n";
        let tags = scan("a.ts", text, SourceLang::Code(CodeLang::TypeScript), &[]);
        assert_eq!(tags.routes[0].methods, ["GET", "POST"]);
        assert_eq!(tags.routes[0].templates, ["/a", "https://h.example/b"]);
        for bad in ["// @route /missing", "// @route GET", "// @route GET a/b", "// @route", "// @route G-T /a"] {
            let tags = scan("a.ts", bad, SourceLang::Code(CodeLang::TypeScript), &[]);
            assert!(tags.routes.is_empty(), "{bad}");
            assert_eq!(tags.malformed.len(), 1, "{bad}");
        }
        let prose = "// the @route tag, and @routes or @router\n";
        let tags = scan("a.ts", prose, SourceLang::Code(CodeLang::TypeScript), &[]);
        assert_eq!(tags, FileTags::default(), "a tag must start its comment line");
    }

    #[test]
    fn contract_syntax() {
        let text = "// @contract a.json#/x/y partial\n// @contract dir/b.json (note)\n// @contract c.json#\n";
        let tags = scan("a.go", text, SourceLang::Code(CodeLang::Go), &[]);
        let got: Vec<_> =
            tags.contracts.iter().map(|c| (c.file_ref.as_str(), c.pointer.as_deref(), c.partial)).collect();
        assert_eq!(got, [("a.json", Some("/x/y"), true), ("dir/b.json", None, false), ("c.json", None, false)]);
        let tags = scan("a.go", "// @contract #/x\n", SourceLang::Code(CodeLang::Go), &[]);
        assert_eq!(tags.malformed.len(), 1);
        // `partial` counts only right after the file.
        let text = "// @contract a.json#/x see the partial docs\n// @contract b.json partial, see notes\n";
        let tags = scan("a.go", text, SourceLang::Code(CodeLang::Go), &[]);
        let got: Vec<bool> = tags.contracts.iter().map(|c| c.partial).collect();
        assert_eq!(got, [false, false], "`partial,` is not the word `partial` either");
    }

    #[test]
    fn digit_separators_do_not_open_char_literals() {
        for lang in [CodeLang::C, CodeLang::Cpp] {
            let text = "int n = 1'000'0; int h = 0x1'FF; // @route GET /a\nchar c = L'x'; // @route GET /b\n";
            let tags = scan("a.cpp", text, SourceLang::Code(lang), &[]);
            assert_eq!(tags.routes.len(), 2, "{lang:?}: {tags:?}");
            let text = "int n = 1'000; char q = '\"'; // @route GET /c\n";
            assert_eq!(scan("a.cpp", text, SourceLang::Code(lang), &[]).routes.len(), 1, "{lang:?}");
        }
    }

    #[test]
    fn raw_triple_quoted_strings_have_no_escapes() {
        // `\` right before the closing quotes ends a Kotlin, Scala or C# raw string.
        let text = "val p = \"\"\"C:\\\"\"\"\n// @route GET /a\nfun f() {}\n";
        for lang in [CodeLang::Kotlin, CodeLang::Scala, CodeLang::CSharp] {
            let tags = scan("a.kt", text, SourceLang::Code(lang), &[]);
            assert_eq!(tags.routes.len(), 1, "{lang:?}: {tags:?}");
        }
        // In Python the backslash escapes the quote: the string goes on and hides the comment.
        let text = "p = \"\"\"C:\\\"\"\"\n# @route GET /a\n\"\"\"\n";
        assert!(scan("a.py", text, SourceLang::Code(CodeLang::Python), &[]).routes.is_empty());
    }

    #[test]
    fn non_code_languages_have_no_tags() {
        let text = "// @route GET /a\n# @route GET /b\n";
        assert_eq!(scan("a.md", text, SourceLang::Markdown, &[]), FileTags::default());
        assert_eq!(scan("a.toml", text, SourceLang::Other, &[]), FileTags::default());
    }

    #[test]
    fn owner_is_the_item_after_the_block() {
        let text =
            "/// Does it.\n/// @route POST /a\n#[allow(dead_code)]\n#[cfg_attr(\n    x,\n    y\n)]\npub fn done() {}\n";
        let tags = scan("a.rs", text, SourceLang::Rust, &[span("done", 8, 8)]);
        assert_eq!(tags.routes[0].owner, TagOwner::Item(span("done", 8, 8)));

        let text = "// @route GET /a\n\nfn later() {}\n";
        let tags = scan("a.rs", text, SourceLang::Rust, &[span("later", 3, 3)]);
        assert_eq!(tags.routes[0].owner, TagOwner::File, "a blank line ends the block");

        let text = "/**\n * Lists.\n * @route GET /things\n */\nexport function list() {}\n";
        let tags = scan("a.ts", text, SourceLang::Code(CodeLang::TypeScript), &[span("list", 5, 5)]);
        assert_eq!(tags.routes[0].line, 3);
        assert_eq!(tags.routes[0].owner, TagOwner::Item(span("list", 5, 5)));

        let text =
            "# @route DELETE /x/{id}\n@app.delete(\"/x/{id}\")\ndef remove(id):\n    \"\"\"@route GET /doc\"\"\"\n";
        let tags = scan("a.py", text, SourceLang::Code(CodeLang::Python), &[span("remove", 3, 4)]);
        assert_eq!(tags.routes.len(), 1, "docstrings are strings");
        assert_eq!(tags.routes[0].owner, TagOwner::Item(span("remove", 3, 4)));
    }

    #[test]
    fn rust_inner_docs_document_their_container() {
        let text = "//! @contract a.json\nfn top() {}\nmod m {\n    //! @contract b.json\n    fn inner() {}\n}\n";
        let owners = [span("top", 2, 2), span("m", 3, 6), span("inner", 5, 5)];
        let tags = scan("a.rs", text, SourceLang::Rust, &owners);
        assert_eq!(tags.contracts[0].owner, TagOwner::File);
        assert_eq!(tags.contracts[1].owner, TagOwner::Item(span("m", 3, 6)));
        // In Enforce `//!` is an ordinary comment: it documents the next item.
        let text = "class A {\n    //! @contract a.json\n    void Send() {}\n}\n";
        let tags = scan("a.c", text, SourceLang::Enforce, &[span("A", 1, 4), span("Send", 3, 3)]);
        assert_eq!(tags.contracts[0].owner, TagOwner::Item(span("Send", 3, 3)));
    }

    #[test]
    fn other_comment_families() {
        let text = "echo \"# @route GET /no\" ${#x} # @route GET /yes\n";
        let tags = scan("a.sh", text, SourceLang::Code(CodeLang::Bash), &[]);
        assert_eq!(tags.routes.len(), 1);
        assert_eq!(tags.routes[0].templates, ["/yes"]);

        let text = "local s = [[\n-- @route GET /no\n]]\n--[[\n @route GET /yes\n]]\n-- @route GET /also\n";
        let tags = scan("a.lua", text, SourceLang::Code(CodeLang::Lua), &[]);
        let lines: Vec<_> = tags.routes.iter().map(|r| r.line).collect();
        assert_eq!(lines, [5, 7]);

        let text = "#[Attr]\n# @route GET /php\n$s = '// @route GET /no';\n";
        let tags = scan("a.php", text, SourceLang::Code(CodeLang::Php), &[]);
        assert_eq!(tags.routes.len(), 1);

        let text = "const s = `\n// @route GET /no\n`;\n/* a /* nested */ still */ // @route GET /yes\n";
        let tags = scan("a.js", text, SourceLang::Code(CodeLang::JavaScript), &[]);
        assert_eq!(tags.routes.len(), 1);
        assert_eq!(tags.routes[0].line, 4);
    }

    #[test]
    fn nested_block_comments_in_rust() {
        let text = "/* outer /* inner */\n @route GET /in */\nfn f() {}\n";
        let tags = scan("a.rs", text, SourceLang::Rust, &[span("f", 3, 3)]);
        assert_eq!(tags.routes.len(), 1);
        assert_eq!(tags.routes[0].line, 2);
        assert_eq!(tags.routes[0].owner, TagOwner::Item(span("f", 3, 3)));
    }
}
