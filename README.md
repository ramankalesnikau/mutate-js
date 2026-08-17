# mutate-js

A Rust-native mutation testing tool for JavaScript/TypeScript, built on [oxc](https://oxc.rs).

Mutation testing injects small deliberate changes ("mutants") into source code and checks
whether the test suite catches them — a stronger signal of test quality than coverage alone.
`mutate-js` is an original implementation for the JS/TS ecosystem: parsing and instrumentation run
natively in Rust (rather than through a JS-hosted transform pipeline), with mutation switching —
embedding every mutant behind a single runtime toggle instead of recompiling per mutant — as the
core architectural bet.

Status: early scaffold. See `crates/` for the workspace layout; the build is being brought up in
stages (parsing → mutant discovery → naive execution → mutation switching → coverage-based test
filtering → incremental caching → config → plugin boundaries → reporting/packaging).

## Development

Requires Rust (stable, via [rustup](https://rustup.rs)) and Node.js 24+.

```bash
cargo build --workspace
cargo test --workspace
```

The native Node binding lives in `crates/mtr-napi`:

```bash
cd crates/mtr-napi
npm install
npm run build:debug
node -e "console.log(require('./index.js').hello())"
```
