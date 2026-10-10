//! Request handlers.

use crate::requests;
use crate::store;

/// Marks a thing done.
/// @route POST /api/v1/things/{id}/done
pub async fn done(body: String) -> &'static str {
    let request = requests::parse_done(&body);
    store::mark(&request);
    "done"
}

/// Starts a thing.
pub async fn start(body: String) -> &'static str {
    #[cfg(feature = "audit")]
    store::audit(&body);
    store::mark(&body);
    "started"
}

/// Shows a thing.
/// @route GET /api/v1/thing/{id}
/// @contract things.schema.json#/definitions/Missing
pub async fn show() -> String {
    String::from("thing")
}

/// Replaces a thing.
pub async fn update(body: String) -> String {
    body.len().to_string()
}

/// One item.
pub async fn item(id: String) -> String {
    id
}

/// Any request for an item.
pub async fn any_item(id: String) -> String {
    id.to_uppercase()
}

/// Empties the store.
pub async fn reset() -> &'static str {
    store::clear();
    "reset"
}

/// Fills the store with sample things.
pub async fn seed() -> &'static str {
    "seeded"
}
