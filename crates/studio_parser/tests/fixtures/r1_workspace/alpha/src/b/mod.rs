pub mod one {
    pub mod two {
        pub fn up() {
            super::super::helper();
        }
    }
}

pub fn helper() {}
