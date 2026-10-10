//! The things crate: its routes are merged into the web API.

pub mod handlers;
mod local;
mod routes;

pub use routes::routes;
