pub mod ast;
pub mod config;
pub mod diagnostic;
pub mod output;
pub mod rules;

/// Build a googlesql (ZetaSQL) parser [`Module`](googlesql::Module) using the
/// backend chosen at compile time.
///
/// By default the self-contained wasmtime engine ([`Module::new`](googlesql::Module::new))
/// is used: it embeds the precompiled ZetaSQL wasm and needs no sidecar, so it
/// builds and runs on every target. With the `native-ffi` feature the parser is
/// instead linked as a prebuilt C-ABI shared library and run as native code
/// ([`Module::new_native_ffi`](googlesql::Module::new_native_ffi)) — faster, but
/// it ships a `libguest_ffi` sidecar and is only prebuilt for some targets.
///
/// Both backends materialize the identical neutral AST, so the choice is
/// invisible past construction.
pub fn build_module() -> Result<googlesql::Module, googlesql::Error> {
    #[cfg(feature = "native-ffi")]
    let module = googlesql::Module::new_native_ffi();
    #[cfg(not(feature = "native-ffi"))]
    let module = googlesql::Module::new();
    module
}
