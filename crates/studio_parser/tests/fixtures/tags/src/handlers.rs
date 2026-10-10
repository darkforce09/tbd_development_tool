//! Handlers for things.
//! @contract things.schema.json#/definitions/Done

use std::fmt::Write;

/// Marks a thing done.
/// @route POST /api/v1/things/:thingId/done
#[allow(dead_code)]
pub async fn done() {
    let s = "// @route GET /nope";
    let mut out = String::new();
    let _ = write!(out, "{s}");
}

// @route /missing-method
fn helper() {}

/// @contract contracts/things.schema.json#/definitions/a~1b partial
pub struct Escaped;
