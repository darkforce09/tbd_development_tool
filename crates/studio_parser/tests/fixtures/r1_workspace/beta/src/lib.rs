//! Beta re-exports alpha's `Thing`.

pub use alpha::Thing;

pub fn beta_run() {
    ::alpha::a::x();
    alpha::prelude::thing();
}

pub fn beta_deps() {
    legacy_core::top();
    legacy_lib::top();
    twin::a::x();
    unixonly::a::x();
    alpha::a::x();
}
