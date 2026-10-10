//! Router functions that exercise statement attributes, early returns and router functions referenced from code
//! the evaluator does not fold into a root.

use axum::routing::get;
use axum::Router;

use crate::handlers;

/// Statement attributes: `#[cfg(test)]` statements are dropped, other `#[cfg]` statements are conditional.
pub fn cfg_routes() -> Router {
    let mut r = Router::new().route("/cfg/always", get(handlers::health));
    #[cfg(test)]
    {
        r = r.route("/test-only/block", get(handlers::health));
    }
    #[cfg(feature = "metrics")]
    {
        r = r.route("/cfg/metrics", get(handlers::health));
    }
    #[cfg(not(test))]
    {
        r = r.route("/cfg/not-test", get(handlers::health));
    }
    #[cfg(debug_assertions)]
    let r = r.route("/cfg/debug", get(handlers::debug::trace));
    #[cfg(test)]
    let r = r.route("/test-only/let", get(handlers::health));
    r
}

/// An early return: what follows exists only when the condition is false.
pub fn early(minimal: bool) -> Router {
    let r = Router::new().route("/early/base", get(handlers::health));
    if minimal {
        return r.route("/early/min", get(handlers::health));
    }
    let r = r.route("/early/full", get(handlers::health));
    r
}

/// An early return the evaluator cannot express as a condition: the function is not evaluated, and the router it
/// nests is not a root.
pub fn tangled(n: u8) -> Router {
    let r = Router::new().nest("/tangled", tangled_inner());
    match n {
        0 => return r,
        _ => {}
    }
    r.route("/tangled/rest", get(handlers::health))
}

fn tangled_inner() -> Router {
    Router::new().route("/inner", get(handlers::health))
}

/// Ends with a value the evaluator does not follow: the router it nests is not a root.
pub fn wrapped() -> Router {
    let r = Router::new().nest("/wrapped", wrapped_inner());
    identity(r)
}

fn identity<T>(value: T) -> T {
    value
}

fn wrapped_inner() -> Router {
    Router::new().route("/inner", get(handlers::health))
}

/// Not a router function (a `Result`): the router it nests is referenced, so it is not a root.
pub fn fallible() -> Result<Router, String> {
    Ok(Router::new().nest("/fallible", fallible_inner()))
}

fn fallible_inner() -> Router {
    Router::new().route("/inner", get(handlers::health))
}

/// Builds a router and never serves it: the router it nests is not a root.
pub fn unserved() {
    let app = Router::new().nest("/unserved", unserved_inner());
    drop(app);
}

fn unserved_inner() -> Router {
    Router::new().route("/inner", get(handlers::health))
}

/// `#[cfg]` twins of one binding, and an `if` whose branches both return.
pub fn twins(admin: bool) -> Router {
    #[cfg(unix)]
    let r = Router::new().route("/twins/unix", get(handlers::health));
    #[cfg(not(unix))]
    let r = Router::new().route("/twins/other", get(handlers::health));
    if admin {
        return r.route("/twins/admin", get(handlers::health));
    } else {
        return r;
    }
}
