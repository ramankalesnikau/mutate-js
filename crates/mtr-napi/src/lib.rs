//! mtr-napi — thin napi-rs binding over `mtr-engine` for Node/TS consumers.
//! Stage 0: proves the Rust-to-npm plumbing works end to end; real engine
//! calls arrive in a later build stage.

#[macro_use]
extern crate napi_derive;

#[napi]
pub fn hello() -> String {
    "hello from mutate-js (native Rust core)".to_string()
}
