//! An integration test with its own router: never part of the route table.

use axum::routing::post;
use axum::Router;

fn stub() -> Router {
    Router::new().route("/things/{id}/done", post(|| async {}))
}

#[test]
fn builds() {
    let _ = stub();
}
