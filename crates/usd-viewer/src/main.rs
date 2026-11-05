//! The USD viewer, which runs in a browser. It starts from `examples/web`, upstream's
//! example, and builds for `wasm32-unknown-unknown` alone.

#[cfg(target_arch = "wasm32")]
mod viewer;

#[cfg(target_arch = "wasm32")]
fn main() {
    viewer::run();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("usd-viewer runs in a browser: build it for wasm32-unknown-unknown");
    std::process::exit(2);
}
