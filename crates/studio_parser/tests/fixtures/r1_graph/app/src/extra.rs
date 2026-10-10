// The whole module is cfg-gated (lib.rs): its calls stay Unresolved.
pub fn extra() {
    crate::store::open();
}
