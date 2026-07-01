use crate::collect::lcov::parse_covered;
use crate::collect::llvm::LlvmTools;
use crate::util::{fnv1a, relativize};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Result of running a single test and exporting its coverage.
pub struct TestCoverage {
    pub status: TestStatus,
    pub duration_ms: u64,
    /// (relative_path, covered_lines) per LCOV record.
    pub records: Vec<(String, Vec<u32>)>,
    /// Captured output (stderr + stdout) of a failing test, so the caller can
    /// surface *why* it failed. `None` for passing tests.
    pub failure_output: Option<String>,
}

#[derive(Clone, Copy)]
pub enum TestStatus {
    Collected,
    Failed,
}

impl TestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TestStatus::Collected => "collected",
            TestStatus::Failed => "failed",
        }
    }
}

/// Per-collect-run constants shared by every test invocation. Bundling them
/// keeps `run_single`'s argument list short and makes the call sites readable.
pub struct RunContext<'a> {
    pub tools: &'a LlvmTools,
    pub staging: &'a Path,
    pub workspace_root: &'a Path,
    /// Every instrumented binary produced by the build.  A test may spawn other
    /// binaries (via `CARGO_BIN_EXE_*`); their coverage lands in the same
    /// profdata but belongs to *different* object files, so `llvm-cov export`
    /// must be told about all of them (via `-object`) to map their source.
    pub objects: &'a [PathBuf],
}

/// Run a single test by exact name with a unique profraw, then export LCOV.
///
/// Every test is run fresh on every collection — there is no result cache.
/// `cwd` is set as the test process's working directory so crate-relative
/// paths resolve as under `cargo test`. `staging` holds only the transient
/// profraw/profdata intermediates needed to produce this test's LCOV.
pub fn run_single(
    target_exe: &Path,
    target_name: &str,
    full_test_path: &str,
    cwd: &Path,
    ctx: &RunContext<'_>,
) -> Result<TestCoverage> {
    let hash = fnv1a(&format!("{target_name}::{full_test_path}"));
    let profdata = ctx.staging.join(format!("{hash}.profdata"));

    // Drop any profraw left over from a previous collection of this same test
    // so we only ever attribute this run's data.
    clean_run_profraw(ctx.staging, &hash);

    let start = Instant::now();
    // `%p` (PID) is essential here.  Tests that spawn other binaries — e.g. via
    // `CARGO_BIN_EXE_<name>` — cause those children to *inherit*
    // LLVM_PROFILE_FILE.  With a fixed filename the parent and child write the
    // same `.profraw` and clobber each other, so coverage that lives only in
    // the spawned binary silently vanishes.  With `%p` each process writes its
    // own `<hash>.<pid>.profraw`; all of them are merged below so the spawned
    // binary's coverage is attributed to this test.
    let profraw_pattern = ctx.staging.join(format!("{hash}.%p.profraw"));
    let output = Command::new(target_exe)
        .current_dir(cwd)
        .arg("--exact")
        .arg(full_test_path)
        .env("LLVM_PROFILE_FILE", &profraw_pattern)
        .output();
    let ok = matches!(&output, Ok(o) if o.status.success());
    let duration_ms = start.elapsed().as_millis() as u64;

    // A failing test still yields (partial) coverage, so a non-zero exit does
    // not abort the run. But the captured output is the only place the actual
    // error/panic lives — retain it so the caller can print it instead of
    // silently burying the failure behind a bare count.
    let failure_output = if ok {
        None
    } else {
        Some(match &output {
            Ok(o) => combine_test_output(&o.stderr, &o.stdout),
            Err(e) => format!("failed to spawn `{}`: {e}", target_exe.display()),
        })
    };

    // Collect every profraw this run wrote: the test process itself plus any
    // spawned children (each substitutes its own PID for `%p`).
    let profraws = collect_run_profraw(ctx.staging, &hash);
    if profraws.is_empty() {
        return Err(anyhow::anyhow!(
            "no coverage from `{}` (binary not instrumented?)",
            full_test_path
        ));
    }
    // Export LCOV even if the test failed — partial coverage is still useful.
    let lcov = export_lcov(ctx.tools, &profraws, &profdata, target_exe, ctx.objects)?;
    let records = relativize_records(parse_covered(&lcov), ctx.workspace_root);

    Ok(TestCoverage {
        status: if ok { TestStatus::Collected } else { TestStatus::Failed },
        duration_ms,
        records,
        failure_output,
    })
}

/// Combine a failing test's stderr and stdout into one string: stderr first
/// (where panics and assertions land), then stdout. Empty halves are dropped.
fn combine_test_output(stderr: &[u8], stdout: &[u8]) -> String {
    let mut out = String::new();
    let stderr = String::from_utf8_lossy(stderr);
    let stdout = String::from_utf8_lossy(stdout);
    if !stderr.trim().is_empty() {
        out.push_str(stderr.trim());
    }
    if !stdout.trim().is_empty() {
        if !out.is_empty() {
            out.push_str("\n--- stdout ---\n");
        }
        out.push_str(stdout.trim());
    }
    out
}

/// Smoke-test that the built binaries are actually coverage-instrumented.
///
/// Runs one test from `target_exe` with a throwaway `LLVM_PROFILE_FILE` and
/// confirms a non-empty `.profraw` is produced. If it isn't, the binaries were
/// built without instrumentation (almost always a stale/polluted target dir
/// that cargo reused) and the whole collection would silently yield nothing.
///
/// `test_path` should be a real test name within `target_exe`.
pub fn check_instrumented(target_exe: &Path, test_path: &str, cwd: &Path) -> Result<()> {
    // A dedicated temp dir + `%p`: the probe test may itself spawn children
    // (via `CARGO_BIN_EXE_*`), and we want each to write its own profraw
    // rather than clobbering one shared file.
    let sink_dir = std::env::temp_dir().join(format!(
        "testmap-smoke-{}-{}",
        std::process::id(),
        fnv1a(test_path),
    ));
    let _ = std::fs::remove_dir_all(&sink_dir);
    let sink_pattern = sink_dir.join("smoke.%p.profraw");
    let ran = Command::new(target_exe)
        .current_dir(cwd)
        .arg("--exact")
        .arg(test_path)
        .env("LLVM_PROFILE_FILE", &sink_pattern)
        .output();
    // The test may pass or fail; we only care whether it (or a child) wrote a
    // non-empty profraw somewhere under the sink dir.
    let ok = matches!(&ran, Ok(_) if has_nonempty_profraw(&sink_dir));
    let _ = std::fs::remove_dir_all(&sink_dir);
    if !ok {
        bail!(
            "test binaries are not coverage-instrumented: running `{}` did not \
             produce a `.profraw`.
  this happens when cargo reuses a stale, non-instrumented build in the \
             coverage target dir.
  fix: delete the coverage build dir ({} under target/testmap/) and re-run \
             `cargo testmap collect`.",
            target_exe.display(),
            target_exe.display()
        );
    }
    Ok(())
}

fn export_lcov(
    tools: &LlvmTools,
    profraws: &[PathBuf],
    profdata: &Path,
    target_exe: &Path,
    objects: &[PathBuf],
) -> Result<String> {
    // Merge *all* profraw files for this run — the test process plus any
    // spawned children — into one profdata.
    let mut merge = Command::new(&tools.profdata);
    merge.arg("merge").arg("-sparse").arg("-o").arg(profdata);
    for p in profraws {
        merge.arg(p);
    }
    let merge = merge.output()?;
    if !merge.status.success() {
        anyhow::bail!(
            "llvm-profdata merge failed: {}",
            String::from_utf8_lossy(&merge.stderr)
        );
    }
    // Export as LCOV.  The primary object is the test binary; every other
    // built binary is passed via `-object` so source files that live in
    // *spawned* binaries (via `CARGO_BIN_EXE_*`) get mapped too.  A binary
    // with no matching records in the profdata is simply ignored by llvm-cov.
    let mut export = Command::new(&tools.cov);
    export
        .arg("export")
        .arg("-format=lcov")
        .arg("-instr-profile")
        .arg(profdata)
        .arg(target_exe);
    for obj in objects {
        if obj != target_exe {
            export.arg("-object").arg(obj);
        }
    }
    let export = export.output()?;
    if !export.status.success() {
        anyhow::bail!(
            "llvm-cov export failed: {}",
            String::from_utf8_lossy(&export.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&export.stdout).into_owned())
}

fn relativize_records(
    records: Vec<(String, Vec<u32>)>,
    workspace_root: &Path,
) -> Vec<(String, Vec<u32>)> {
    let mut out = Vec::with_capacity(records.len());
    for (abs, lines) in records {
        let p = PathBuf::from(&abs);
        let Some(rel) = relativize(&p, workspace_root) else {
            // Out-of-workspace file (e.g. std/core) — ignore.
            continue;
        };
        out.push((rel.to_string_lossy().into_owned(), lines));
    }
    out
}

/// Remove every `<hash>.*.profraw` in `staging` left over from a previous
/// collection of this same test.  Keeps each run's data self-contained.
fn clean_run_profraw(staging: &Path, hash: &str) {
    for p in glob_run_profraw(staging, hash) {
        let _ = std::fs::remove_file(p);
    }
}

/// Collect every `<hash>.*.profraw` in `staging` written during this run —
/// the test process itself plus any spawned children (each substitutes its
/// PID for `%p`).
fn collect_run_profraw(staging: &Path, hash: &str) -> Vec<PathBuf> {
    glob_run_profraw(staging, hash)
}

fn glob_run_profraw(staging: &Path, hash: &str) -> Vec<PathBuf> {
    let prefix = format!("{hash}.");
    staging
        .read_dir()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|e| {
            let name = e.file_name();
            name.to_string_lossy().starts_with(&prefix)
                && name.to_string_lossy().ends_with(".profraw")
        })
        .map(|e| e.path())
        .collect()
}

/// True if `dir` contains any non-empty `.profraw`.
fn has_nonempty_profraw(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name().to_string_lossy().ends_with(".profraw")
                && std::fs::metadata(e.path()).map(|m| m.len() > 0).unwrap_or(false)
        })
}

