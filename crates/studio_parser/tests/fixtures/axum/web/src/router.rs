use axum::routing::{get, post};
use axum::Router;
use tower_http::services::ServeDir;

use crate::handlers;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let dev = state.dev;
    let mut r = Router::new()
        .route("/healthz", get(handlers::health))
        .nest("/api/v1", api_routes(dev));
    if dev {
        r = r.route("/debug", get(handlers::debug::dump));
        if cfg!(debug_assertions) {
            r = r.route("/debug/trace", get(handlers::debug::trace));
        }
    }
    r = r.nest_service("/assets", ServeDir::new("assets"));
    r.with_state(state)
}

fn api_routes(dev: bool) -> Router<AppState> {
    let mut api = Router::new().merge(things::routes());
    if dev {
        api = api.route("/dev/reset", post(handlers::debug::reset));
    } else {
        api = api.route("/status", get(handlers::health));
    }
    for path in ["/a", "/b"] {
        api = api.route(path, get(handlers::health));
    }
    api
}
