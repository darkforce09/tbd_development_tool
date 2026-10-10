// `use` imports both name spaces: this leaf names the fn `tidy` (helpers.rs) and the module `tidy` (helpers/tidy.rs),
// two files, so it stays Unresolved and keeps its name match.
use crate::helpers::tidy;
// Test code: R1 does not read it; the name match is kept and not counted.
#[cfg(test)]
use crate::store;
