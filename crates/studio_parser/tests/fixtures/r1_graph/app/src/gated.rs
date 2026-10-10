// The caller is cfg-gated: the call resolves, but stays Unresolved.
#[cfg(feature = "extra")]
pub fn gated() {
    crate::store::flush();
}
