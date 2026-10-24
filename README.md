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

## Configuration

`mutate.config.jsonc` (see `mutate.config.example.jsonc`) controls the run. Notable options:

- `testRunner` — `"jest"` or `"vitest"`.
- `testFilter` — `"all"` (whole suite), `"related"` (the runner's import-graph resolver), or
  `"coverage"` (per-test statement coverage, the most precise).
- `dryRun` (default `true`) — run the suite unmutated first and abort if it fails, since a red
  suite makes every mutant result meaningless.
- `concurrency` (default `1`) — mutants tested concurrently within a file. Opt-in because each
  Jest/Vitest run already fans out to its own worker pool.
- `excludedMutators` / `mutatorPacks` — narrow or widen the operator catalogue.

Mutants can be suppressed at the source site with `// mutate-js disable-next-line`,
`// mutate-js disable-line`, and `// mutate-js disable` / `// mutate-js restore`, each optionally
followed by operator names (`// mutate-js disable-next-line StringLiteral`).

## Development

Requires Rust (stable, via [rustup](https://rustup.rs)) and Node.js 24+.

```bash
cargo build --workspace
cargo test --workspace
```

The native Node binding lives in `crates/mtr-napi`, a thin wrapper over the
`mtr-core` engine. Results cross the FFI boundary as JSON strings:

```bash
cd crates/mtr-napi
npm install
npm run build:debug
node -e "console.log(require('./index.js').scan('src/**/*.js', false))"
```
