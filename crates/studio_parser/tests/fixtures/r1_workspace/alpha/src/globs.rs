pub mod one {
    pub fn only_one() {}
    pub fn shared() {}
    pub fn shadowed() {}
}

pub mod two {
    pub fn shared() {}
    pub fn shadowed() {}
}

pub mod user {
    use super::one::*;
    use super::two::*;
    use super::two::shadowed;

    pub fn run() {
        only_one();
        shared();
        shadowed();
    }
}
