use std::env;

fn main() {
    // napi-build: link the node addon symbols.
    napi_build::setup();
    let _ = env::var("CARGO_PKG_NAME"); // silence unused-import lint in older toolchains
}
