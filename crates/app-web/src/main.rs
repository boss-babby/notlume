//! WebAssembly entry point. Build with `trunk build --release` from this
//! directory (see README). On native targets this binary only prints a hint,
//! so `cargo build --workspace` keeps working.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("app-web targets wasm32-unknown-unknown. Run `trunk serve` or `trunk build --release` in crates/app-web.");
}

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}
