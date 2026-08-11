//! Wires up the run-time search paths for the `native-ffi` backend's sidecar.
//!
//! The default (wasmtime) backend is self-contained and needs nothing here. Under
//! the `native-ffi` feature the binary links googlesql's prebuilt `libguest_ffi`
//! C-ABI shared library, whose reference is relocatable (`@rpath` install name /
//! bare `SONAME`). Resolving it at run time is the consumer's job: a dependency's
//! build script cannot bake an rpath into this binary's link. googlesql publishes
//! the directory it resolved the cdylib to as `DEP_GUEST_FFI_LIBDIR` (via its
//! `links = "guest_ffi"`); we turn that into two rpaths — one for development
//! (`cargo run`/`cargo test`) and one for a shipped binary that carries a copy of
//! the library next to itself. See googlesql's `docs/NATIVE.md`.

fn main() {
    // Only the native-ffi backend links the sidecar; the wasmtime default embeds
    // the parser and needs no search paths. Skipping the rpath args otherwise
    // also keeps them off targets that never use native-ffi (e.g. Windows, where
    // `-Wl,-rpath` is meaningless).
    if std::env::var_os("CARGO_FEATURE_NATIVE_FFI").is_none() {
        return;
    }
    println!("cargo::rerun-if-env-changed=DEP_GUEST_FFI_LIBDIR");

    // Development: load the cdylib from wherever googlesql resolved it (OUT_DIR on
    // the download path, or a local GUEST_FFI_LIB). Only a direct dependent of
    // googlesql receives this — bqvalid depends on it directly, so it is set.
    if let Ok(dir) = std::env::var("DEP_GUEST_FFI_LIBDIR") {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
    }

    // Shipped binary: load a copy of libguest_ffi.{dylib,so} placed next to the
    // executable (the release archive bundles it — see `.goreleaser.yaml`).
    let origin = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        "@executable_path"
    } else {
        "$ORIGIN"
    };
    println!("cargo::rustc-link-arg=-Wl,-rpath,{origin}");
}
