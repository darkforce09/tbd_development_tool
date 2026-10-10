//! The things API.

pub mod handlers;
pub mod requests;
pub mod store;

use axum::routing::{any, get, post};
use axum::Router;

/// The API version.
pub fn version() -> &'static str {
    "1"
}

/// The whole API; the development routes only when `dev`, the seed route when `dev` or `demo`. The admin routes
/// are merged under `demo` and then always, so they are unconditional.
pub fn router(dev: bool, demo: bool) -> Router {
    let mut app = Router::new().route("/health", get(|| async { "ok" })).nest("/api/v1", api_v1());
    if demo {
        app = app.merge(admin());
    }
    app = app.merge(admin());
    if dev {
        app = app.route("/dev/reset", post(handlers::reset));
        app = app.merge(seed());
    }
    if demo {
        app = app.merge(seed());
    }
    app
}

fn seed() -> Router {
    Router::new().route("/seed", post(handlers::seed))
}

fn api_v1() -> Router {
    Router::new()
        .route("/things/{id}/done", post(handlers::done))
        .route("/things/{id}/start", post(handlers::start))
        .route("/things/{id}", get(handlers::show).post(handlers::update))
        .route("/items/{id}", get(handlers::item))
}

fn admin() -> Router {
    Router::new().route("/api/v1/items/{name}", any(handlers::any_item))
}

/// Routes that exist only with the `extra` feature.
#[cfg(feature = "extra")]
pub mod extra;
