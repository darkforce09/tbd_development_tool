//! Prints the API version and asks the API to start a thing again.

/// A program that is also a sender: it stays a program.
/// @route POST /api/v1/things/{id}/start
fn main() {
    let version = server::version();
    println!("{version}");
    server::requests::resend("1");
}
