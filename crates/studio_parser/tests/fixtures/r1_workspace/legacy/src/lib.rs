//! An edition 2015 library: `use` paths and `::name` start at the crate root.

pub mod helper {
    pub fn assist() {}
}

pub mod sub {
    use helper::assist;

    pub fn run() {
        assist();
        ::helper::assist();
        ::std::mem::drop(1);
    }
}

pub fn top() {}
