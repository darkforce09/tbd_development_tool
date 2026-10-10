use axum::Router;

/// Not axum's `get`: a lookup that shares the name.
pub fn get(key: &str) -> Option<&'static str> {
    match key {
        "a" => Some("b"),
        _ => None,
    }
}

pub fn lookup_twice() -> usize {
    get("a").map_or(0, str::len) + get("b").map_or(0, str::len)
}

pub fn local_routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new().route("/local", get("a"))
}
