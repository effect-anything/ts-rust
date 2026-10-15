fn main() {
    napi_build::setup();
    // `napi_build::setup` lets the cdylib leave the `napi_*` symbols to the
    // Node process that loads it. The `tsc-ext` binary, the examples and the test
    // executables link the same rlib, which references those symbols from
    // the Node API module even though they never call it; on macOS the
    // linker refuses them. Resolve them lazily there too: nothing outside a
    // Node process reaches that code.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for kind in ["bins", "tests", "examples"] {
            println!("cargo:rustc-link-arg-{kind}=-Wl,-undefined,dynamic_lookup");
        }
    }
}
