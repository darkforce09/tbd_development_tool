//! Routes that exist only with the `extra` feature: the module is cfg-gated, so the calls in this file are too.

use axum::routing::post;
use axum::Router;

/// The extra routes.
pub fn extra_router() -> Router {
    Router::new().route("/extra/clear", post(clear_all))
}

/// Empties the store; the call is in a gated module, so it is no Calls link.
pub async fn clear_all() -> &'static str {
    crate::store::clear();
    "cleared"
}
