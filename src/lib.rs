pub mod ast;
pub mod config;
pub mod diagnostic;
pub mod output;
pub mod rules;

/// Build a googlesql (ZetaSQL) parser [`Module`](googlesql::Module).
///
/// bqvalid uses googlesql's `native-ffi` backend exclusively
/// ([`Module::new_native_ffi`](googlesql::Module::new_native_ffi)): the parser is
/// linked as a prebuilt `libguest_ffi` C-ABI shared library and run as native
/// code. The sidecar ships next to the binary in the release archive (see
/// `.goreleaser.yaml`), and `build.rs` wires the rpath that finds it. Only the
/// two targets with a prebuilt sidecar are supported: `x86_64-unknown-linux-gnu`
/// and `aarch64-apple-darwin`.
pub fn build_module() -> Result<googlesql::Module, googlesql::Error> {
    googlesql::Module::new_native_ffi()
}
