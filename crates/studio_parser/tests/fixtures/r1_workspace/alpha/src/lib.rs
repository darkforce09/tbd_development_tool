//! Alpha: the base library of the R1 fixture.

use axum::Router;

pub mod a;
pub mod b;
pub mod c;
pub mod globs;
pub mod inner;
pub mod prelude;
pub mod shapes;
#[path = "other.rs"]
pub mod p;

#[cfg(feature = "x")]
pub fn h() {}
#[cfg(not(feature = "x"))]
pub fn h() {}

macro_rules! m {
    () => { 0 };
}

pub struct Thing;

impl Thing {
    pub fn new() -> Self {
        Thing
    }

    pub fn make() -> Self {
        Self::new()
    }
}

pub fn entry() {
    prelude::thing();
    self::inner::thing();
    crate::a::x();
    p::hidden();
    h();
    let _ = m!();
    Thing::new();
    shapes::Shape::Circle(1);
    let v = Vec::new();
    v.len();
    let f = || 1;
    f();
    tests::helper();
    Router::new();
}

#[cfg(test)]
mod tests {
    pub fn helper() {}

    #[test]
    fn calls_are_not_read_here() {
        helper();
    }
}
