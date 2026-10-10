use axum::routing::{any, get, on, post, MethodFilter};
use axum::Router;

use crate::handlers;

const OTHER: &str = "/other";

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/things/{thingId}/done", post(handlers::things::done))
        .route(
            "/things",
            get(handlers::things::list).post(handlers::things::create),
        )
        .route("/x", axum::routing::delete(handlers::things::remove))
        .route("/ping", get(|| async { "pong" }))
        .route(OTHER, get(handlers::things::list))
        .route(
            "/things/{id}",
            on(MethodFilter::PUT, handlers::things::replace).layer(tower::layer::util::Identity::new()),
        )
        .route("/any", any(handlers::things::list))
        .merge(crate::local::local_routes())
}
