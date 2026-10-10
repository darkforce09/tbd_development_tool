//! The web crate: application state, handlers and the router.

pub mod handlers;
pub mod prelude;
pub mod router;
mod state;

pub use state::AppState;

#[cfg(test)]
mod t {
    use axum::routing::get;
    use axum::Router;

    pub fn test_router() -> Router {
        Router::new().route("/test-only", get(super::handlers::health))
    }
}

pub mod more;

/// Compiled only with the `extra` feature: its routes carry the condition.
#[cfg(feature = "extra")]
pub mod extra;
