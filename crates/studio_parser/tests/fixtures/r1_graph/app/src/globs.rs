// Both globs provide `run`: ambiguous, so the call keeps the name match, Unresolved. The glob imports themselves
// name one module each.
use crate::alpha::*;
use crate::beta::*;

pub fn globs() {
    run();
}
