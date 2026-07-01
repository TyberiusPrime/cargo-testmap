# cargo-testmap — Design Document

## 1. Goal

`cargo-testmap` is a cargo subcommand that answers the question: **"Which tests cover this line of code?"**

Unlike traditional coverage tools (which tell you "is this line covered?"),
`cargo-testmap` builds a **reverse map**: for every line of source code, it
tells you the *set of specific test functions* that exercise that line. It then
presents this as a browsable, syntax-highlighted HTML report.

## 2. High-Level Architecture

```
┌─────────────────────┐     ┌──────────────────────┐     ┌─────────────────────┐
│  cargo testmap      │     │  cargo testmap        │     │  cargo testmap       │
│  collect            │────▶│  database (JSON)      │────▶│  report              │
│                     │     │  testmap.json         │     │                      │
│  Runs each test     │     │                      │     │  Reads DB only        │
│  with llvm-cov,     │     │  line → [test_names]  │     │  Generates HTML      │
│  extracts coverage  │     │                      │     │  with highlighting   │
└─────────────────────┘     └──────────────────────┘     └─────────────────────┘
```

Two-phase design with a JSON database as the boundary. This lets you iterate on
the report independently of the expensive collection phase.

## 3. Collection Phase (`cargo testmap collect`)

### 3.1 Strategy: Per-Test-Function Coverage

The core idea: run `cargo llvm-cov` once per individual test function, capturing only the coverage that test produces.

**How it works:**

1. **Build once with instrumentation.** Use `cargo llvm-cov show-env --sh` to get the required env vars, then `cargo test --no-run --message-format=json` to build all test binaries.

2. **Enumerate test binaries & their tests.** From step 1's JSON output, extract the list of test binaries (path, name, kind). For **each binary individually**, run `<binary> -- --list --format terse` to get that binary's test names. This gives us the critical test↔binary mapping — a single test name like `test_foo` may exist in multiple binaries, and we must track which binary owns which instance.

3. **Run each test individually.** For each test in each binary:
   - Set `LLVM_PROFILE_FILE` to a unique per-test profraw path
   - Run `<binary> --exact <test_name>` directly
   - **Important:** `LLVM_PROFILE_FILE` must be unique per test, not per binary. Use a deterministic name derived from the test name (not PID — PIDs are not safe with parallelism: two tests from the same binary running concurrently must not collide).

4. **Export per-test coverage as LCOV.** For each test's profraw:
   - `llvm-profdata merge -sparse <test>.profraw -o <test>.profdata`
   - `llvm-cov export -format=lcov -instr-profile <test>.profdata <binary> > <test>.lcov`

5. **Read source files** referenced by the LCOV data and snapshot their content into the database (see §4.1).

6. **Accumulate into a reverse map.** For each test's LCOV, extract the set of `(file, line)` pairs where `count > 0`. Accumulate: `map[(file, line)].insert(test_index)`.

7. **Apply threshold filter.** Remove entries where `len(test_indices) >= threshold`.

8. **Write the database.** Serialize the test lookup table + source snapshots + filtered map to JSON.

### 3.2 Test Enumeration Details

`<binary> -- --list --format terse` outputs lines like:
```
test path::to::module::test_name ... ok
```

We need to:
- Parse these into test names
- Group by test binary (each binary is run separately)
- Handle unit tests, integration tests, and doc tests separately

**Critical: run `--list` per binary, not globally.** `cargo test -- --list` concatenates all binaries' test lists into one output with no indication of which binary owns which test. Two binaries can have tests with the same `test_name` but different `module::path` prefixes — and sometimes even identical names. Running `--list` per binary gives us unambiguous ownership.

Test binaries are discovered via step 1's `cargo test --no-run --message-format=json`, which gives us JSON objects with `target.name`, `target.kind`, and `filenames[]`.

### 3.3 Threshold Filtering (Omit)

The "omit if more than X tests" filter is applied **during database construction**, not in the HTML viewer.

**How it works:** After accumulating all per-test coverage into the reverse map, before writing the database, we remove any `(file, line)` entries where `len(test_indices) >= threshold`.

- **Default threshold:** `10`
- **Configurable** via `--threshold <N>` on `cargo testmap collect`
- The resulting database contains only "interesting" lines — those with few enough tests to be worth inspecting
- Uncovered lines (0 tests) are also excluded (implicitly uncovered)

**Why on the collection side:** The viewer should be dumb. It just renders what's in the database. Changing the threshold means re-generating the database (cheap — just re-process the accumulated map, no need to re-run tests if we keep the raw intermediate data, see §3.4).

### 3.4 Performance Considerations

**This is O(n) invocations where n = number of tests.** For a project with 500 tests, that's 500 separate runs.

Mitigations:
- **Build once, run many:** Build all test binaries once with instrumentation, then run each test by invoking the binary directly.
- **Parallel execution:** Run multiple test processes in parallel (controllable with `--jobs`).
- **Filtering:** Allow `--filter <pattern>` to collect only matching tests.
- **Skip list:** Allow `--skip <pattern>` to skip tests.

**No test-result caching.** Every `collect` runs every matched test from
scratch — there is no resume cache, no fingerprinting, no stale-data risk.
Result caching turned out to be a source of subtle correctness bugs (stale
coverage after a non-source edit like a test fixture, schema drift, TOCTOU
between the build and the run) for little gain, so it was removed. Cargo's
own build cache still handles the expensive part (reusing the instrumented
binaries when nothing changed, rebuilding them when it did); only the per-test
*run* is repeated. If your tests are slow, that is the cost of always-correct
coverage.

### 3.5 Concrete Implementation Approach

```
Step 1: Build with instrumentation
  $ eval "$(cargo llvm-cov show-env --sh)"
  $ cargo test --no-run --message-format json
  → Parse: list of binaries, each with path/name/kind

Step 2: Enumerate tests per binary
  For each binary:
    $ <binary> -- --list --format terse
  → Parse: list of test names, associated with this binary

Step 3: Run each test individually (with parallelism via --jobs)
  For each test in each binary:
    $ LLVM_PROFILE_FILE=target/llvm-cov-target/testmap/<binary_name>_<test_name_hash>.profraw \
        <binary> --exact <test_name>

Step 4: Export per-test LCOV
  For each test's profraw:
    $ llvm-profdata merge -sparse <test>.profraw -o <test>.profdata
    $ llvm-cov export -format=lcov -instr-profile <test>.profdata <binary> > <test>.lcov

Step 5: Read source files & parse LCOV
  Collect all unique file paths from LCOV SF: records.
  Read and snapshot each source file.
  Parse all <test>.lcov: DA:<line>,<count> → covered if count > 0.
  Accumulate reverse map: (file, line) → [test_index].

Step 6: Apply threshold, write testmap.json
```

## 4. Database Format

### 4.1 Schema (testmap.json)

Tests are referenced by index to avoid repeating full test name strings in every line entry.

```json
{
  "version": 1,
  "metadata": {
    "generated_at": "2026-06-30T22:00:00Z",
    "workspace_root": "/path/to/project",
    "cargo_testmap_version": "0.1.0",
    "collection_args": ["--workspace", "--lib"]
  },
  "tests": [
    {
      "name": "test_foo",
      "module": "mycrate::parser",
      "binary": "test_lib",
      "kind": "unit",
      "status": "collected",
      "duration_ms": 12
    },
    {
      "name": "test_foo",
      "module": "tests::integration",
      "binary": "integration",
      "kind": "integration",
      "status": "collected",
      "duration_ms": 34
    }
  ],
  "sources": {
    "src/lib.rs": {
      "content": "use std::collections::HashMap;\n\npub fn process(data: &Data) -> Result<Output> {\n    let mut map = HashMap::new();\n    // ...\n}\n",
      "lines": {
        "3": [0, 1],
        "5": [0]
      }
    },
    "src/parser.rs": {
      "content": "pub fn parse(input: &str) -> Result<Ast> {\n    // ...\n}\n",
      "lines": {
        "1": [1]
      }
    }
  }
}
```

### 4.2 Design Rationale

- **`tests` is an array** (not a map). Test metadata lives once here; line entries reference tests by **array index**. This avoids repeating test name strings across potentially thousands of line entries. A 200-test project with 30K covered lines at avg 5 tests/line goes from ~4.5 MB of repeated strings to ~6 KB of lookup + compact integer arrays.
- **Each test includes `module`** (the full path from `--list`, e.g. `mycrate::parser::test_foo`) and **`binary`** (the test binary name). This disambiguates identically-named tests from different binaries — the viewer shows both so the user knows which `test_foo` is which.
- **`sources[file].content` contains the full source text** (snapshotted at collection time). This makes the database fully self-contained: the report generator never reads source files from disk, so later source changes don't invalidate the report. Paths in `sources` keys are relative to `workspace_root`.
- **`lines` keys are strings** (JSON doesn't support integer keys). Values are arrays of **test indices**.
- **Only covered lines are stored** — lines with zero tests are implicitly uncovered. Lines with `>= threshold` tests are also excluded at this stage (see §3.3).
- **No `total_lines` count** — derivable from `content` (line count), no need to duplicate.
- **`metadata`** captures the collection context for reproducibility.
- **`version` field** for forward-compatible schema evolution.

### 4.3 Size Estimation

For a medium project (100 files × 500 lines avg, 200 tests, ~60% coverage, threshold=10):
- Tests array: ~200 × ~100 bytes = ~20 KB
- Source snapshots: 100 files × ~15 KB avg = ~1.5 MB
- ~30,000 covered lines × ~15 bytes per entry (line key + few integer indices) ≈ 450 KB
- After threshold filtering, many lines with 10+ tests are dropped, likely cutting this in half
- Total: ~1-2 MB. Still very manageable for JSON.

## 5. Report Generation Phase (`cargo testmap report`)

### 5.1 Input

- `testmap.json` (the database — contains everything, including source snapshots)
- Configuration (theme, output dir, include/exclude filters)

No source files are read from disk.

### 5.2 Output

A directory of self-contained HTML files (no server needed):

```
testmap-report/
├── index.html              # File listing / navigation
├── css/
│   └── style.css           # Theme & layout
├── js/
│   └── app.js              # Hover/click interactivity (minimal)
├── src/
│   ├── lib.rs.html         # Per-file annotated source view
│   └── parser.rs.html
└── data/
    └── coverage.js         # Per-file coverage data embedded as JS
```

### 5.3 Viewer Design: Hover + Click

The viewer is deliberately minimal. The only interactivity is:

**Layout:** Vertical split — source on top, test panel below (full width):
```
┌─────────────────────────────────────────────────────────┐
│  src/lib.rs                                              │
├─────────────────────────────────────────────────────────┤
│  1 │ use std::collections::HashMap;                     │
│  2 │                                                     │
│  3 │ pub fn process(data: &Data) -> Result<Output> {     │
│  4 │     let mut map = HashMap::new();                   │
│  5 │ ►   if data.is_empty() {                           │
│  6 │         return Err(Error::Empty);                    │
│  7 │ ►   }                                               │
│  8 │     map.insert(data.key(), data.value());           │
│  ...                                                     │
├─────────────────────────────────────────────────────────┤
│  Tests covering line 5 (2 tests):                        │
│    • [unit/test_lib] mycrate::parser::test_process_empty  │
│    • [integration/integration] tests::integration::test_foo│
│                                                          │
│  [click a line to pin · click again to unpin]            │
└─────────────────────────────────────────────────────────┘
```

Full width for both panes accommodates long Rust test names like
`test_some_module_some_struct_some_trait_impl_some_behavior`.

**Test disambiguation:** Each test entry in the panel shows its `[binary_kind/binary_name]` prefix and full module path, so identically-named tests from different binaries are distinguishable.

**Hover:**
- Hovering over any line that has coverage data shows the covering tests in the panel below
- The hovered line gets a subtle highlight
- Moving the mouse away clears the panel and highlight

**Click (freeze/unfreeze):**
- Clicking a line **freezes** the selection: the line stays highlighted, the test panel stays populated
- Clicking the **same line again** unfreezes (clears selection)
- Clicking a **different line** moves the frozen selection to the new line

**Lines not in the database** (uncovered or threshold-omitted): hovering shows nothing in the panel, clicking has no effect.

### 5.4 Per-File View

Each source file gets an HTML page with:

1. **Syntax-highlighted source code** (Syntect, build-time) with line numbers, full width
2. **Test panel below** the source, full width, shows covering tests on hover/click
3. **Minimal gutter indicator** on lines that have coverage data (e.g. a small dot or color accent) so the user can visually scan for annotated lines at a glance
4. **File path** displayed at the top

### 5.5 Index Page

- Simple file listing (links to per-file views)
- Optionally grouped by directory
- No dashboard metrics (keep it simple)

### 5.6 Syntax Highlighting

**Syntect** (build-time):
- Generate `<span class="...">` tokens with CSS classes during report generation
- Theme applied via a single CSS file (swappable)
- No JS dependency for rendering
- Fully self-contained HTML files, no CDN needed
- The only JS is the minimal hover/click handler

## 6. Configuration

### 6.1 Config File: `.testmap.toml` (in project root)

```toml
[collect]
# Which test targets to collect
targets = ["lib", "tests"]  # "lib", "bin", "tests", "benches", "examples", "doc"
# Test name filter (regex)
filter = "test_.*"
# Test name skip (regex)
skip = "test_slow_.*"
# Parallelism
jobs = 4

[report]
# Output directory
output_dir = "target/testmap-report"
# Syntax highlighting theme
theme = "Catppuccin Mocha"  # or "One Dark", "GitHub Dark", etc.
# Source paths to include (relative to workspace root)
include = ["src/"]
# Source paths to exclude
exclude = ["src/generated/"]
```

### 6.2 CLI Override

All config values can be overridden on the command line:

```bash
cargo testmap collect --workspace --lib --filter "test_parse_*" --jobs 8 --threshold 5
cargo testmap report --output-dir ./coverage --theme "Monokai"
```

## 7. Dependency & Crate Choices

| Need | Crate | Why |
|------|-------|-----|
| CLI framework | `clap` | Standard, feature-rich |
| JSON parsing | `serde` + `serde_json` | Standard |
| TOML config | `serde` + `toml` | Standard |
| Syntax highlighting | `syntect` | Mature, many themes, pure Rust |
| Process execution | `std::process::Command` | No need for extra crate |
| Parallel execution | `rayon` or `tokio` | For running tests concurrently |
| HTML templating | `askama` or hand-written | Depends on complexity |
| File discovery | `ignore` (from ripgrep) | Handle .gitignore properly |
| Progress reporting | `indicatif` | Nice progress bars |
| Test binary discovery | Parse `cargo test --no-run --message-format json` + per-binary `-- --list` | Standard cargo JSON output |

No need for the `llvm-cov-json` crate — we parse LCOV ourselves (trivial parser).

## 8. Workspace / Project Layout

```
cargo-testmap/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI entry point, subcommand dispatch
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── collect.rs     # `collect` subcommand
│   │   └── report.rs     # `report` subcommand
│   ├── collect/
│   │   ├── mod.rs
│   │   ├── test_discovery.rs   # Enumerate tests & binaries
│   │   ├── coverage_run.rs     # Run a single test with coverage
│   │   ├── coverage_parse.rs   # Parse llvm-cov JSON/lcov output
│   │   └── database.rs         # Accumulate & write testmap.json
│   ├── report/
│   │   ├── mod.rs
│   │   ├── database.rs         # Read testmap.json
│   │   ├── source.rs           # Highlight source from DB snapshot
│   │   ├── html/
│   │   │   ├── mod.rs
│   │   │   ├── index.rs        # Dashboard page
│   │   │   ├── file_view.rs    # Per-file annotated view
│   │   │   ├── css.rs          # Generate CSS
│   │   │   └── js.rs           # Generate JS
│   │   └── config.rs
│   └── config.rs           # .testmap.toml parsing
```

## 9. Remaining Open Questions

### 9.1 Granularity: Per-Test-Function vs Per-Test-Binary

Running per individual test function is the ideal but very slow for large projects.

**Decision: Per-test-function only for now.** If needed, a `--granularity binary` fast mode can be added later, but per-test-function is the whole point of this tool.

### 9.2 No Result Caching (every collect runs every test)

**Decision: no resume / no result cache.** Every `cargo testmap collect`
re-runs every matched test from scratch and rewrites the database. An earlier
design proposed a staging-dir resume cache, but it was removed because it was
a net source of subtle staleness bugs (coverage/status served from a previous
run after the code, tests, or runtime fixtures like `test_cases/` changed) and
the fingerprinting needed to make it correct can't see non-binary inputs at
all. Cargo's build cache still reuses the instrumented binaries when nothing
changed, so only the (correct, always-fresh) per-test execution repeats. The
staging dir now holds only the transient `profraw`/`profdata` intermediates
needed to export a test's LCOV within a single run.

### 9.3 Coverage Format: LCOV

**Decision: Use LCOV.** Simple `DA:<line>,<count>` entries. Trivial to parse (~20 lines of code). No need for the `llvm-cov-json` crate.

### 9.4 LCOV Parsing

LCOV format relevant records:
```
SF:/absolute/path/to/src/lib.rs      # source file start
DA:42,1                                # line 42, execution count 1 (covered)
DA:43,0                                # line 43, execution count 0 (not covered)
DA:44,5                                # line 44, execution count 5 (covered)
end_of_record                          # source file end
```

A line is considered **covered** by the test if `count > 0`. Trivial to parse: ~20 lines of Rust.

### 9.5 Test Failure Handling

If a test fails (panics, assertions fail):
- Still capture the coverage data (the test ran, after all — partial coverage is useful)
- Mark the test as `status: "failed"` in the database
- The report can visually distinguish failed tests

### 9.6 Workspace Support

For Cargo workspaces:
- Collect coverage across all workspace members (default)
- `--package <spec>` to limit to specific packages
- The database should include the workspace root for path resolution

### 9.7 HTML: Single File vs Multi-File

**Multi-file (recommended):**
- Per-file pages keep each HTML small and fast to load
- Shared CSS/JS reduces duplication
- Better for large projects

**Single-file alternative:**
- Useful for sharing a report as one file
- Could be offered as `cargo testmap report --single-file output.html`
- Embeds all data and sources inline

## 10. CLI Interface Summary

```
# Collect coverage data (builds DB)
cargo testmap collect [OPTIONS]
  --workspace              # All workspace members (default)
  -p, --package <SPEC>     # Specific package
  --lib / --tests / --bins # Target filter
  --filter <PATTERN>       # Only collect tests matching regex
  --skip <PATTERN>         # Skip tests matching regex
  --threshold <N>           # Omit lines covered by >=N tests (default: 10)
  -j, --jobs <N>           # Parallel test runs (default: num_cpus)
  --output <PATH>          # Database output path (default: target/testmap/testmap.json)
  -v, --verbose            # Verbose output

# Generate HTML report from DB
cargo testmap report [OPTIONS]
  --input <PATH>           # Database input path (default: target/testmap/testmap.json)
  --output-dir <DIR>       # Report output (default: target/testmap/report)
  --theme <NAME>           # Syntax highlighting theme
  --include <PATH>         # Only include matching source paths
  --exclude <PATH>         # Exclude matching source paths
  --single-file <PATH>     # Generate a single HTML file instead of directory
```

## 11. Implementation Phases

### Phase 1: MVP
- `collect` subcommand (show-env approach: build once, run each test individually)
- LCOV parsing for per-test line coverage
- JSON database with deduplicated test lookup table
- Threshold filtering during collection
- `report` subcommand with per-file HTML, Syntect syntax highlighting
- Minimal viewer: hover to see tests in panel below, click to freeze
- Simple index page with file listing

### Phase 2: Performance
- Parallel test execution (`--jobs`)
- The instrumented build is reused by cargo's own cache (fast when unchanged);
  per-test results are never cached — every collect runs every test fresh.

### Phase 3: Polish
- `.testmap.toml` configuration
- Workspace support (`--package`, `--workspace`)
- Source include/exclude filters
- Single-file report mode

### Phase 4: Extras
- Doc test support
- CI integration (summary output, fail-on-low-coverage)
- VS Code extension (use DB to show coverage in editor)
