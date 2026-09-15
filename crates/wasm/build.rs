//! Link the web build's module with a stack of two megabytes.
//!
//! rust-lld gives a wasm module one megabyte of stack unless told otherwise,
//! and a read that runs out of it takes the whole module down. The depth
//! guards in the evaluator keep a read within 640 KiB of it (`STACK_BUDGET`
//! in `eval/go.rs`), and that stays what they are measured against: the
//! second megabyte is room over for whoever called in and for a shape nobody
//! has measured, not a reason to let a read go deeper.
//!
//! Set here rather than as a flag in `.cargo/config.toml` so that it applies
//! to this module alone, however cargo is run, and a `RUSTFLAGS` in the
//! environment does not quietly replace it. The stack is the first part of
//! the module's memory, so the memory it starts with grows by the same
//! megabyte; the file does not.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-cdylib=-zstack-size=2097152");
    }
}
