//! Compiled only with the `extra` feature (`#[cfg(feature = "extra")] pub mod extra;`).

use axum::routing::get;
use axum::Router;

pub fn extra_routes() -> Router {
    let r = Router::new().route("/extra", get(crate::handlers::health));
    #[cfg(unix)]
    let r = r.route("/extra/unix", get(crate::handlers::health));
    r
}

#[cfg(target_os = "linux")]
pub fn linux_routes() -> Router {
    Router::new().route("/extra/linux", get(crate::handlers::health))
}
