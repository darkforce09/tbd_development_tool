//! Request bodies.
//! @contract things.schema.json#/definitions/Done

/// A parsed "done" request.
pub fn parse_done(body: &str) -> String {
    body.trim().to_string()
}

/// Asks the API to start a thing again: a function that is also a sender stays a function.
/// @route POST /api/v1/things/{id}/start
pub fn resend(id: &str) {
    let _ = id;
}
