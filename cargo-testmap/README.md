# cargo-testmap

`cargo-testmap` answers the question: **which tests cover this line of code?**

Unlike a normal coverage tool (which tells you *whether* a line is covered),
`cargo-testmap` builds a **reverse map**: for every line of source code, it
tells you the *set of specific test functions* that exercise that line, and
renders it as a browsable, syntax-highlighted HTML report.

See [`DESIGN.md`](./DESIGN.md) for the full design document.


# Screenshots

![Screenshot showing overview](https://raw.githubusercontent.com/TyberiusPrime/cargo-testmap/main/cargo-testmap/showcase/index.png)

![Screenshot showing example file](https://raw.githubusercontent.com/TyberiusPrime/cargo-testmap/main/cargo-testmap/showcase/file.png)

## How it works

**`cargo testmap run`** - builds & runs every test under coverage, individually,
collects the data into $CARGO_TARGET_DIR/testmap and builds the report.


## Split invokation

Two phases, with a self-contained JSON database (`testmap.json`) as the
boundary — so the devs can iterate on the report without re-running the expensive
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

## Coverage rules & the index

The report's top-level `index.html` lists a **total coverable line count**
(`covered / coverable (pct)`), per-file and summed across the whole project, so
files missing coverage stand out instead of being hidden behind a bare line
count. Files are ordered worst-coverage-first.

Every executable source line is classified into one category, shown as a
colored dot in its gutter (hover the dot for an explanation; the index also has
a legend). The **uncovered** and **ignored** counts in a stats line are
links — click them to jump to the next such line (on the index they jump into
the file). Uncovered lines get a red dot so gaps are obvious at a glance.

| Dot color | Category      | Meaning                                                       |
|-----------|---------------|---------------------------------------------------------------|
| green     | covered       | reached by at least one test                                  |
| red       | uncovered     | a real gap — no test reached it                               |
| white     | excluded      | coverage is *not expected* (see markers / `unreachable!`)     |
| pink      | excl-covered  | excluded but covered anyway — the marker is probably stale    |
| grey      | ignored       | panic-shaped noise (`unwrap`/`expect`/`panic!`/…) or markers  |

The classification is computed from the database alone (source text + the
executable-line set + covered lines), so re-running `report` after tweaking
the rules needs no re-collection.

### Markers

**Excluded** — coverage is explicitly *not expected* (removed from the
coverable total; white dot):

- `//cov:excl-start` … `//cov:excl-stop` — exclude the lines in the region.
- `//cov:excl-line` — exclude just this line.
- `unreachable!` — any line containing `unreachable!` is auto-excluded.

**Ignored** — panic-shaped noise we don't want muddying the numbers (grey dot;
not counted as coverable):

- `//cov:ignore-start` … `//cov:ignore-stop` — ignore the lines in the region.
- `//cov:ignore-line` — ignore just this line.
- Panic sites: `panic!`, `.unwrap()`, `.expect(`, `todo!`, `unimplemented!`.
  (`unreachable!` is *excluded*, not ignored.)

Region markers (`-start`/`-stop`) may appear as `//cov:excl-start` or with a
space (`// cov:excl-start`). A line carrying any cov marker is itself
classified by that marker (an `excl-*` line is excluded, an `ignore-*` line is
ignored) — so a marker comment that llvm-cov spuriously instruments (a nearby
`format!(` macro bleeding a counter onto it) won't show up red. Trailing
commentary after a marker is fine (`//cov:excl-start reason…`) — a line carries
at most one marker role (whichever marker appears first), so commentary that
merely *mentions* another marker (e.g. `//cov:excl-start … closed by
cov:excl-stop`) won't be mistaken for it. Non-`unwrap()` variants like
`unwrap_or` are *not* ignored (they don't panic).

## Requirements

- A Rust toolchain with the `llvm-tools-preview` component (provides
  `llvm-cov` / `llvm-profdata`).
- [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) on your `PATH`
  (used only for its `show-env` helper that sets up instrumentation).

## Usage

```sh
cargo testmap run                             # → target/testmap/testmap.json & target/testmap/report/index.html

# or in two steps
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


# Notes on the HTML

Dot color code:
 * green - covered
 * red - not covered
 * grey - ignored 
 * purple - excluded, but 
 * no dot - irrelevant.

Once you've clicked on 'uncovered/excluded/ignored', you can press space to jump to the next one!

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
