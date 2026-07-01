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

/// Cache (meta.json) schema version. Bump whenever the on-disk shape of a
/// staged result changes (e.g. a field is added). A cached `meta.json` whose
/// version doesn't match is treated as a miss and the test is re-collected,
/// so a stale staging dir from an older cargo-testmap can't silently drop
/// fields (this is how `failure_output` originally went missing).
///
/// v2 adds `binary_fingerprint`: a content hash of the test binary, checked
/// on load so editing code/tests (cargo rebuild) invalidates the entry
/// instead of serving stale coverage.
const META_VERSION: u32 = 2;

/// Per-collect-run constants shared by every test invocation. Bundling them
/// keeps `run_single`'s argument list short and makes the call sites readable.
pub struct RunContext<'a> {
    pub tools: &'a LlvmTools,
    pub staging: &'a Path,
    pub workspace_root: &'a Path,
    pub clean: bool,
}

/// Run a single test by exact name with a unique profraw, then export LCOV.
///
/// `staging` holds per-test intermediate files (profraw/profdata/lcov/meta) for
/// incremental resume. `binary_fingerprint` invalidates the resume cache when
/// the binary is rebuilt (code/tests changed). `cwd` is set as the test
/// process's working directory so crate-relative paths work as under `cargo test`.
pub fn run_single(
    target_exe: &Path,
    target_name: &str,
    full_test_path: &str,
    binary_fingerprint: &str,
    cwd: &Path,
    ctx: &RunContext<'_>,
) -> Result<TestCoverage> {
    let hash = fnv1a(&format!("{target_name}::{full_test_path}"));
    let lcov_path = ctx.staging.join(format!("{hash}.lcov"));
    let meta_path = ctx.staging.join(format!("{hash}.meta.json"));
    let profraw = ctx.staging.join(format!("{hash}.profraw"));
    let profdata = ctx.staging.join(format!("{hash}.profdata"));

    // Resume: reuse staged results when not forcing a clean collection.
    if !ctx.clean && lcov_path.exists() && meta_path.exists()
        && let Some(cov) = load_cached(&lcov_path, &meta_path, ctx.workspace_root, binary_fingerprint) {
            return Ok(cov);
        }

    let start = Instant::now();
    let output = Command::new(target_exe)
        .current_dir(cwd)
        .arg("--exact")
        .arg(full_test_path)
        // A unique, deterministic profile file — no PID, safe under parallelism.
        .env("LLVM_PROFILE_FILE", &profraw)
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

    // Export LCOV even if the test failed — partial coverage is still useful.
    // If the profraw is missing, the binary wasn't instrumented for this run;
    // surface that instead of silently recording empty coverage.
    let lcov = match export_lcov(ctx.tools, &profraw, &profdata, target_exe) {
        Ok(s) => s,
        Err(e) => {
            // Clean up so resume doesn't cache a bogus empty result.
            let _ = std::fs::remove_file(&lcov_path);
            let _ = std::fs::remove_file(&meta_path);
            return Err(e.context(format!(
                "no coverage from `{}` (binary not instrumented?)",
                full_test_path
            )));
        }
    };
    let records = relativize_records(parse_covered(&lcov), ctx.workspace_root);

    // Stage results for resume.
    let _ = std::fs::write(&lcov_path, &lcov);
    let _ = std::fs::write(
        &meta_path,
        serde_json::json!({
            "version": META_VERSION,
            "binary_fingerprint": binary_fingerprint,
            "status": if ok { "collected" } else { "failed" },
            "duration_ms": duration_ms,
            "failure_output": failure_output,
        })
        .to_string(),
    );

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
    let sink = std::env::temp_dir().join(format!(
        "testmap-smoke-{}-{}.profraw",
        std::process::id(),
        fnv1a(test_path),
    ));
    let _ = std::fs::remove_file(&sink);
    let ran = Command::new(target_exe)
        .current_dir(cwd)
        .arg("--exact")
        .arg(test_path)
        .env("LLVM_PROFILE_FILE", &sink)
        .output();
    // The test may pass or fail; we only care whether it wrote a profraw.
    let ok = match &ran {
        Ok(_) => sink.exists() && std::fs::metadata(&sink).map(|m| m.len() > 0).unwrap_or(false),
        Err(_) => false,
    };
    let _ = std::fs::remove_file(&sink);
    if !ok {
        bail!(
            "test binaries are not coverage-instrumented: running `{}` did not \
             produce a `.profraw`.\n  this happens when cargo reuses a stale, \
             non-instrumented build in the coverage target dir.\n  fix: re-run \
             `cargo testmap collect --clean` (it wipes the coverage build dir).",
            target_exe.display()
        );
    }
    Ok(())
}

fn export_lcov(
    tools: &LlvmTools,
    profraw: &Path,
    profdata: &Path,
    target_exe: &Path,
) -> Result<String> {
    // Merge the single profraw.
    let merge = Command::new(&tools.profdata)
        .arg("merge")
        .arg("-sparse")
        .arg(profraw)
        .arg("-o")
        .arg(profdata)
        .output()?;
    if !merge.status.success() {
        anyhow::bail!(
            "llvm-profdata merge failed: {}",
            String::from_utf8_lossy(&merge.stderr)
        );
    }
    // Export as LCOV.
    let export = Command::new(&tools.cov)
        .arg("export")
        .arg("-format=lcov")
        .arg("-instr-profile")
        .arg(profdata)
        .arg(target_exe)
        .output()?;
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

#[derive(serde::Deserialize)]
struct Meta {
    /// `None` on caches written before versioning existed → treated as a miss.
    #[serde(default)]
    version: Option<u32>,
    /// Content hash of the binary this entry was collected against. Compared
    /// to the current binary's fingerprint on load; mismatch → re-collect.
    #[serde(default)]
    binary_fingerprint: String,
    status: String,
    duration_ms: u64,
    #[serde(default)]
    failure_output: Option<String>,
}

fn load_cached(
    lcov_path: &Path,
    meta_path: &Path,
    workspace_root: &Path,
    expected_fingerprint: &str,
) -> Option<TestCoverage> {
    let lcov = std::fs::read_to_string(lcov_path).ok()?;
    let meta: Meta = serde_json::from_str(&std::fs::read_to_string(meta_path).ok()?).ok()?;
    // Refuse to reuse a cache whose schema predates the current one: a missing
    // or mismatched version means fields we now rely on (e.g. failure_output)
    // may be absent. Fall through to a fresh collection instead.
    if meta.version != Some(META_VERSION) {
        return None;
    }
    // Invalidate when the test binary changed since this entry was written —
    // i.e. the code or tests were edited and cargo rebuilt. Without this,
    // resume would silently serve coverage (and pass/fail status) from the
    // previous binary.
    if !expected_fingerprint.is_empty() && meta.binary_fingerprint != expected_fingerprint {
        return None;
    }
    let status = if meta.status == "failed" {
        TestStatus::Failed
    } else {
        TestStatus::Collected
    };
    Some(TestCoverage {
        status,
        duration_ms: meta.duration_ms,
        records: relativize_records(parse_covered(&lcov), workspace_root),
        failure_output: meta.failure_output.filter(|s| !s.trim().is_empty()),
    })
}
