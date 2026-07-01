use crate::collect::llvm::coverage_env;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// An instrumented test binary discovered from `cargo test --no-run`.
#[derive(Debug, Clone)]
pub struct TestTarget {
    pub name: String,
    pub kind: String,
    pub executable: PathBuf,
    /// The package directory (parent of the owning Cargo.toml). Used as the
    /// working directory when running the test binary, mirroring `cargo test`,
    /// so tests that open files via paths relative to their crate
    /// (e.g. `"../test_cases/..."`) resolve identically.
    pub cwd: PathBuf,
}

/// A single test function within a test target.
#[derive(Debug, Clone)]
pub struct TestCase {
    /// Index into the discovered targets vector.
    pub target_index: usize,
    /// Full path as reported by `--list`, e.g. `parser::tests::test_foo`.
    pub full: String,
    /// Bare test name (last `::` segment, or the whole thing if unqualified).
    pub name: String,
    /// Module path (everything before the final `::`, possibly empty).
    pub module: String,
}

/// Build all test binaries once with instrumentation, in an isolated target
/// directory `cov_target_dir`. Returns the discovered test binaries.
pub fn build_targets(
    dir: &Path,
    cargo_args: &[String],
    cov_target_dir: &str,
    verbose: bool,
) -> Result<Vec<TestTarget>> {
    let cov_env = coverage_env(dir, Some(cov_target_dir))?;
    let targets = build_and_collect(dir, cargo_args, &cov_env, cov_target_dir, verbose)?;
    if targets.is_empty() {
        anyhow::bail!("no test binaries were produced; is there a `#[test]` in the project?");
    }
    Ok(targets)
}

#[derive(Deserialize)]
struct Artifact {
    reason: Option<String>,
    target: Option<Target>,
    profile: Option<Profile>,
    executable: Option<String>,
    /// Absolute path to the owning Cargo.toml; its parent is the package
    /// directory `cargo test` runs the binary from.
    manifest_path: Option<String>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct Profile {
    test: Option<bool>,
}

fn build_and_collect(
    dir: &Path,
    cargo_args: &[String],
    cov_env: &BTreeMap<String, String>,
    cov_target_dir: &str,
    verbose: bool,
) -> Result<Vec<TestTarget>> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(dir);
    cmd.args(["test", "--no-run", "--message-format", "json"]);
    // Isolate the instrumented build so it can never be silently reused from
    // a prior non-instrumented build in the project's normal target dir.
    cmd.env("CARGO_TARGET_DIR", cov_target_dir);
    for (k, v) in cov_env {
        cmd.env(k, v);
    }
    cmd.args(cargo_args);
    // Capture stdout (JSON) but stream stderr so build warnings/errors show.
    let output = cmd
        .output()
        .context("running `cargo test --no-run --message-format json`")?;
    if !output.status.success() {
        std::io::Write::write_all(&mut std::io::stderr(), &output.stderr).ok();
        anyhow::bail!("instrumented build failed");
    }
    if verbose {
        std::io::Write::write_all(&mut std::io::stderr(), &output.stderr).ok();
    }

    // The last `kind` element is the most specific (e.g. ["lib"] vs the
    // integration-style target name).
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in output.stdout.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(art) = serde_json::from_slice::<Artifact>(line) else {
            continue;
        };
        if art.reason.as_deref() != Some("compiler-artifact") {
            continue;
        }
        let Some(target) = art.target else { continue };
        let is_test = art
            .profile
            .as_ref()
            .and_then(|p| p.test)
            .unwrap_or(false);
        if !is_test {
            continue;
        }
        let Some(exe) = art.executable else { continue };
        if !seen.insert(exe.clone()) {
            continue;
        }
        let kind = target.kind.last().cloned().unwrap_or_default();
        let executable = PathBuf::from(&exe);
        // Package directory = parent of Cargo.toml. Falls back to the
        // invocation dir only if cargo omitted the field (it never does in
        // practice), so we still run rather than fail.
        let cwd = art
            .manifest_path
            .as_deref()
            .map(PathBuf::from)
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| dir.to_path_buf());
        targets.push(TestTarget {
            name: target.name,
            kind,
            executable,
            cwd,
        });
    }

    Ok(targets)
}

/// Enumerate the tests within a single binary via `--list --format terse`.
/// See DESIGN §3.2.
pub fn list_tests(target: &TestTarget) -> Result<Vec<TestCase>> {
    // `--list` still executes (and exits) the instrumented binary, which would
    // otherwise dump a stray `default.%p.profraw` into the project dir. Point
    // it at a throwaway temp file instead.
    let sink = std::env::temp_dir().join(format!(
        "testmap-list-{}-{}.profraw",
        std::process::id(),
        crate::util::fnv1a(&target.executable.to_string_lossy()),
    ));
    let out = Command::new(&target.executable)
        .current_dir(&target.cwd)
        .args(["--list", "--format", "terse"])
        .env("LLVM_PROFILE_FILE", &sink)
        .output()
        .with_context(|| format!("listing tests in {}", target.executable.display()))?;
    if !out.status.success() {
        anyhow::bail!(
            "`{} --list` failed:\n{}",
            target.executable.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut cases = Vec::new();
    for line in text.lines() {
        // Format: `path::to::test_name: test`
        let Some((path, kind)) = line.rsplit_once(": ") else {
            continue;
        };
        if kind.trim() != "test" {
            continue;
        }
        let path = path.trim();
        let (module, name) = match path.rsplit_once("::") {
            Some((m, n)) => (m.to_string(), n.to_string()),
            None => (String::new(), path.to_string()),
        };
        cases.push(TestCase {
            target_index: 0, // filled in by caller
            full: path.to_string(),
            name,
            module,
        });
    }
    Ok(cases)
}
