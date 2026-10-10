// `run` is defined in alpha.rs and beta.rs; the name match picks beta.rs, R1 follows the import to alpha.rs.
use crate::alpha::run;

pub fn retargeted() {
    run();
}
