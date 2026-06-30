# cargo-testmap

`cargo-testmap` answers the question: **which tests cover this line of code?**

Unlike a normal coverage tool (which tells you *whether* a line is covered),
`cargo-testmap` builds a **reverse map**: for every line of source code, it
tells you the *set of specific test functions* that exercise that line, and
renders it as a browsable, syntax-highlighted HTML report.

See [`DESIGN.md`](./DESIGN.md) for the full design document.

## How it works

Two phases, with a self-contained JSON database (`testmap.json`) as the
boundary — so you can iterate on the report without re-running the expensive
collection.

1. **`cargo testmap collect`** — builds every test binary once with coverage
   instrumentation, then runs each individual test function to capture *its*
   coverage, and accumulates a `(file, line) → [tests]` reverse map. Lines
   covered by too many tests (configurable threshold) are omitted.
2. **`cargo testmap report`** — reads only the database (it never touches your
   source files again) and emits a self-contained HTML report with
   syntax-highlighted source and a hover/click test panel. The report has a
   dark/light theme toggle behind a hamburger menu (top-right); the choice is
   remembered across pages.

## Requirements

- A Rust toolchain with the `llvm-tools-preview` component (provides
  `llvm-cov` / `llvm-profdata`).
- [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) on your `PATH`
  (used only for its `show-env` helper that sets up instrumentation).

## Usage

```sh
# from inside a crate that has tests:
cargo testmap collect --lib --tests           # → target/testmap/testmap.json
cargo testmap report                          # → target/testmap/report/index.html

# options:
cargo testmap collect --filter 'parse_' --skip 'slow_' --threshold 5 --jobs 8
cargo testmap report --theme 'base16-mocha.dark'
cargo testmap report --single-file coverage.html   # one self-contained file
```

A trivial example target lives in [`../example`](../example) — see its
[`README`](../example/README.md).

## Project layout

```
src/
├── main.rs              CLI entry + cargo-subcommand dispatch
├── cli.rs               clap definitions
├── config.rs            optional .testmap.toml
├── util.rs              path helpers, timestamps, hashing
├── collect/             collection phase
│   ├── llvm.rs            tool discovery + show-env
│   ├── test_discovery.rs  build + per-binary test enumeration
│   ├── coverage_run.rs    run one test, export LCOV
│   ├── lcov.rs            minimal LCOV parser
│   └── database.rs        reverse map + testmap.json schema
└── report/              report phase
    ├── database.rs        read testmap.json
    ├── highlight.rs       syntect line highlighting
    ├── html.rs            index + per-file + single-file rendering
    ├── style.css          report theme
    └── app.js             hover/click interactivity
```
