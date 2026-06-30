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

/// Run a single test by exact name with a unique profraw, then export LCOV.
///
/// `staging` holds per-test intermediate files (profraw/profdata/lcov/meta) for
/// incremental resume. `cache_key` uniquely identifies this (binary, test).
pub fn run_single(
    target_exe: &Path,
    target_name: &str,
    full_test_path: &str,
    tools: &LlvmTools,
    staging: &Path,
    workspace_root: &Path,
    clean: bool,
) -> Result<TestCoverage> {
    let hash = fnv1a(&format!("{target_name}::{full_test_path}"));
    let lcov_path = staging.join(format!("{hash}.lcov"));
    let meta_path = staging.join(format!("{hash}.meta.json"));
    let profraw = staging.join(format!("{hash}.profraw"));
    let profdata = staging.join(format!("{hash}.profdata"));

    // Resume: reuse staged results when not forcing a clean collection.
    if !clean && lcov_path.exists() && meta_path.exists()
        && let Some(cov) = load_cached(&lcov_path, &meta_path, workspace_root) {
            return Ok(cov);
        }

    let start = Instant::now();
    let status = Command::new(target_exe)
        .arg("--exact")
        .arg(full_test_path)
        // A unique, deterministic profile file — no PID, safe under parallelism.
        .env("LLVM_PROFILE_FILE", &profraw)
        .output();
    let ok = matches!(status, Ok(o) if o.status.success());
    let duration_ms = start.elapsed().as_millis() as u64;

    // Export LCOV even if the test failed — partial coverage is still useful.
    // If the profraw is missing, the binary wasn't instrumented for this run;
    // surface that instead of silently recording empty coverage.
    let lcov = match export_lcov(tools, &profraw, &profdata, target_exe) {
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
    let records = relativize_records(parse_covered(&lcov), workspace_root);

    // Stage results for resume.
    let _ = std::fs::write(&lcov_path, &lcov);
    let _ = std::fs::write(
        &meta_path,
        serde_json::json!({
            "status": if ok { "collected" } else { "failed" },
            "duration_ms": duration_ms,
        })
        .to_string(),
    );

    Ok(TestCoverage {
        status: if ok { TestStatus::Collected } else { TestStatus::Failed },
        duration_ms,
        records,
    })
}

/// Smoke-test that the built binaries are actually coverage-instrumented.
///
/// Runs one test from `target_exe` with a throwaway `LLVM_PROFILE_FILE` and
/// confirms a non-empty `.profraw` is produced. If it isn't, the binaries were
/// built without instrumentation (almost always a stale/polluted target dir
/// that cargo reused) and the whole collection would silently yield nothing.
///
/// `test_path` should be a real test name within `target_exe`.
pub fn check_instrumented(target_exe: &Path, test_path: &str) -> Result<()> {
    let sink = std::env::temp_dir().join(format!(
        "testmap-smoke-{}-{}.profraw",
        std::process::id(),
        fnv1a(test_path),
    ));
    let _ = std::fs::remove_file(&sink);
    let ran = Command::new(target_exe)
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
    status: String,
    duration_ms: u64,
}

fn load_cached(
    lcov_path: &Path,
    meta_path: &Path,
    workspace_root: &Path,
) -> Option<TestCoverage> {
    let lcov = std::fs::read_to_string(lcov_path).ok()?;
    let meta: Meta = serde_json::from_str(&std::fs::read_to_string(meta_path).ok()?).ok()?;
    let status = if meta.status == "failed" {
        TestStatus::Failed
    } else {
        TestStatus::Collected
    };
    Some(TestCoverage {
        status,
        duration_ms: meta.duration_ms,
        records: relativize_records(parse_covered(&lcov), workspace_root),
    })
}
