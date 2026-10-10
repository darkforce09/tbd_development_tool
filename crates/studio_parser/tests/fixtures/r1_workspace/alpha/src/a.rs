pub fn x() {}

pub fn b() {}

pub fn g() {}

pub fn run() {
    inner::f();
}

mod inner {
    pub fn f() {
        super::g();
    }
}

// A statement-level macro naming `x` may declare it in the body.
pub fn with_macro() {
    declare!(x);
    x();
}
