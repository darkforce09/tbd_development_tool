pub mod d;

use crate::a::{self, b as c};

pub fn cee() {
    a::x();
    c();
    self::local();
}

fn local() {}
