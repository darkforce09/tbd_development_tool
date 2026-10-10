//! HTTP route templates as written in router code and `@route` tags, reduced to segments that can be compared.
//!
//! Parameter names never matter: `/things/:thingId` and `/things/{id}` are the same route. A parameter is a whole
//! segment: `{name}` or `:name` (letters, digits, `_`). Literals compare exactly (case sensitive); `{{` and `}}` are
//! escaped braces. Wildcard segments (`*rest`, `{*rest}`, `:rest*`, `:rest+`) are kept so they can be counted, but a
//! template that holds one never unifies with anything: which requests a wildcard catches depends on the router,
//! not the text. Any other segment with a parameter marker in it (`{a}-{b}`, `:id.json`, `:id?`, an unbalanced
//! brace) is [`Segment::Opaque`]: its meaning depends on the router, so it never unifies either.

/// One `/`-separated piece of a route template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Segment {
    /// Text that must match byte for byte.
    Literal(String),
    /// `:name` or `{name}`: matches any single segment.
    Param,
    /// `*name` or `{*name}`: matches the rest of the path.
    Wildcard,
    /// A segment that mixes parameters and text, or uses router-specific syntax (`{a}-{b}`, `:id?`), as written.
    Opaque(String),
}

/// A route path split into segments. The root path `/` has no segments.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RouteTemplate {
    pub segments: Vec<Segment>,
}

impl RouteTemplate {
    /// Parses a template. `?query` and `#fragment` are dropped first; then a full URL (`scheme://host…` at the start)
    /// loses its scheme and host; empty segments are skipped, so a trailing `/` (or a doubled one) changes nothing.
    pub fn parse(s: &str) -> RouteTemplate {
        let mut path = s.trim();
        if let Some(cut) = query_start(path) {
            path = &path[..cut];
        }
        if let Some(scheme_end) = scheme_len(path) {
            let after = &path[scheme_end + 3..];
            path = after.find('/').map_or("", |slash| &after[slash..]);
        }
        let segments = path.split('/').filter(|part| !part.is_empty()).map(segment).collect();
        RouteTemplate { segments }
    }

    /// The path a `rest` route gets when its router is nested under `prefix` (axum `Router::nest`).
    pub fn join(prefix: &RouteTemplate, rest: &RouteTemplate) -> RouteTemplate {
        let mut segments = prefix.segments.clone();
        segments.extend(rest.segments.iter().cloned());
        RouteTemplate { segments }
    }

    /// True when both templates describe the same requests: same length, equal literals, a parameter against a
    /// parameter. A wildcard or opaque segment on either side never unifies.
    pub fn unifies(&self, other: &RouteTemplate) -> bool {
        self.segments.len() == other.segments.len()
            && self.segments.iter().zip(&other.segments).all(|pair| match pair {
                (Segment::Literal(a), Segment::Literal(b)) => a == b,
                (Segment::Param, Segment::Param) => true,
                _ => false,
            })
    }

    /// True when any segment is a wildcard.
    pub fn has_wildcard(&self) -> bool {
        self.segments.contains(&Segment::Wildcard)
    }

    /// The template with parameter names erased: `/a/{}/b`, wildcards as `{*}`, the root as `/`. Braces inside a
    /// literal are written escaped (`{{`, `}}`); an opaque segment is written as it was.
    pub fn canonical(&self) -> String {
        if self.segments.is_empty() {
            return "/".to_string();
        }
        let mut out = String::new();
        for segment in &self.segments {
            out.push('/');
            match segment {
                Segment::Literal(text) => out.push_str(&text.replace('{', "{{").replace('}', "}}")),
                Segment::Param => out.push_str("{}"),
                Segment::Wildcard => out.push_str("{*}"),
                Segment::Opaque(text) => out.push_str(text),
            }
        }
        out
    }
}

/// Where `?query` or `#fragment` starts. A `?` that ends a `:name` segment (`/things/:id?`, an optional parameter
/// in some routers) is part of that segment.
fn query_start(path: &str) -> Option<usize> {
    let bytes = path.as_bytes();
    path.match_indices(['?', '#']).map(|(i, _)| i).find(|&i| {
        if bytes[i] == b'#' {
            return true;
        }
        let segment = &path[path[..i].rfind('/').map_or(0, |s| s + 1)..i];
        let ends_segment = matches!(bytes.get(i + 1), None | Some(b'/'));
        !(ends_segment && segment.starts_with(':'))
    })
}

/// The length of a leading URL scheme (`https` in `https://…`), when the text starts with one.
fn scheme_len(path: &str) -> Option<usize> {
    let end = path.find("://")?;
    let scheme = &path[..end];
    let mut chars = scheme.chars();
    let starts_alpha = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    (starts_alpha && chars.all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))).then_some(end)
}

fn is_name(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn segment(part: &str) -> Segment {
    let opaque = || Segment::Opaque(part.to_string());
    if let Some(name) = part.strip_prefix(':') {
        return match name.strip_suffix(['*', '+']) {
            Some(name) if is_name(name) => Segment::Wildcard,
            _ if is_name(name) => Segment::Param,
            _ => opaque(),
        };
    }
    if let Some(rest) = part.strip_prefix('*') {
        return if rest.is_empty() || is_name(rest) { Segment::Wildcard } else { opaque() };
    }
    if let Some(inner) = part.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        if let Some(name) = inner.strip_prefix('*') {
            return if is_name(name) { Segment::Wildcard } else { opaque() };
        }
        if is_name(inner) {
            return Segment::Param;
        }
    }
    // A literal, with `{{` / `}}` standing for one brace; any other brace is router syntax.
    let mut literal = String::with_capacity(part.len());
    let mut chars = part.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' if chars.peek() == Some(&c) => {
                chars.next();
                literal.push(c);
            }
            '{' | '}' => return opaque(),
            c => literal.push(c),
        }
    }
    Segment::Literal(literal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> RouteTemplate {
        RouteTemplate::parse(s)
    }

    #[test]
    fn braces_and_colons_are_the_same_parameter() {
        assert!(t("/things/{x}/done").unifies(&t("/things/:y/done")));
        assert_eq!(t("/things/{x}/done").canonical(), "/things/{}/done");
        assert_eq!(t("/things/:y/done").canonical(), "/things/{}/done");
    }

    #[test]
    fn literals_compare_exactly() {
        assert!(!t("/things/{x}/done").unifies(&t("/things/{x}/start")));
        assert!(!t("/Things").unifies(&t("/things")));
        assert!(!t("/things/{x}").unifies(&t("/things/done")), "a parameter is not a literal");
    }

    #[test]
    fn different_lengths_never_unify() {
        assert!(!t("/things").unifies(&t("/things/{id}")));
        assert!(!t("/").unifies(&t("/things")));
        assert!(t("/").unifies(&t("")));
    }

    #[test]
    fn wildcards_never_unify() {
        assert_eq!(t("/files/*rest").segments[1], Segment::Wildcard);
        assert_eq!(t("/files/{*rest}").segments[1], Segment::Wildcard);
        assert!(!t("/files/*rest").unifies(&t("/files/*rest")));
        assert!(!t("/files/{*rest}").unifies(&t("/files/{x}")));
        assert!(t("/files/{*rest}").has_wildcard());
        assert_eq!(t("/files/{*rest}").canonical(), "/files/{*}");
    }

    #[test]
    fn join_is_nest() {
        let joined = RouteTemplate::join(&t("/api/v1"), &t("/things/{id}"));
        assert_eq!(joined.canonical(), "/api/v1/things/{}");
        assert!(joined.unifies(&t("/api/v1/things/:thingId")));
        assert_eq!(RouteTemplate::join(&t("/api"), &t("/")).canonical(), "/api");
    }

    #[test]
    fn full_urls_queries_and_fragments_are_stripped() {
        assert_eq!(t("https://example.com:8080/api/v1/things?limit=5").canonical(), "/api/v1/things");
        assert_eq!(t("http://localhost").canonical(), "/");
        assert_eq!(t("/things/{id}#top").canonical(), "/things/{}");
        assert_eq!(t("/things?x=/y").canonical(), "/things");
    }

    #[test]
    fn a_url_inside_the_query_is_not_the_path() {
        assert_eq!(t("/cb?next=https://h/x").canonical(), "/cb");
        assert_eq!(t("/cb#https://h/x").canonical(), "/cb");
        // A scheme only counts at the start.
        assert_eq!(t("/a/x://b/c").canonical(), "/a/x:/b/c");
        assert_eq!(t("9p://h/x").canonical(), "/9p:/h/x");
    }

    #[test]
    fn parameters_are_whole_segments_only() {
        for mixed in ["{a}-{b}", ":id.json", ":id?", "{id", "x}", "{a b}", "pre{id}", "{}", ":", "*a.b"] {
            let template = t(&format!("/f/{mixed}"));
            assert_eq!(template.segments[1], Segment::Opaque(mixed.to_string()), "{mixed}");
            assert!(!template.unifies(&template), "{mixed} unifies with itself");
            assert!(!template.unifies(&t("/f/{x}")), "{mixed} unifies with a parameter");
        }
        assert_eq!(t("/f/:id*").segments[1], Segment::Wildcard);
        assert_eq!(t("/f/:id+").segments[1], Segment::Wildcard);
        assert_eq!(t("/f/{id_2}").segments[1], Segment::Param);
        // Escaped braces are a literal.
        assert_eq!(t("/f/{{literal}}").segments[1], Segment::Literal("{literal}".to_string()));
        assert_eq!(t("/f/{{literal}}").canonical(), "/f/{{literal}}");
        assert!(t("/f/{{literal}}").unifies(&t("/f/{{literal}}")));
        assert!(!t("/f/{{literal}}").unifies(&t("/f/{x}")));
    }

    #[test]
    fn trailing_and_doubled_slashes_are_ignored() {
        assert_eq!(t("/things/"), t("/things"));
        assert_eq!(t("//things//done"), t("/things/done"));
        assert!(t("/").segments.is_empty());
    }
}
