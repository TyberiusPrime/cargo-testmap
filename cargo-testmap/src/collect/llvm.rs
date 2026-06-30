use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Paths to the llvm tools bundled with the active Rust toolchain, plus the
/// cargo-llvm-cov helper. See DESIGN §3.5 step 1 & 4.
pub struct LlvmTools {
    pub profdata: PathBuf,
    pub cov: PathBuf,
}

impl LlvmTools {
    pub fn discover() -> Result<Self> {
        let sysroot = rustc_sysroot()?;
        let host = rustc_host()?;
        let bin = sysroot
            .join("lib")
            .join("rustlib")
            .join(&host)
            .join("bin");
        let profdata = bin.join("llvm-profdata");
        let cov = bin.join("llvm-cov");
        if !profdata.exists() || !cov.exists() {
            bail!(
                "could not find llvm-profdata/llvm-cov under {}\n\
                 hint: install the `llvm-tools-preview` rustup component",
                bin.display()
            );
        }
        Ok(Self { profdata, cov })
    }
}

fn rustc_sysroot() -> Result<PathBuf> {
    let out = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .context("running `rustc --print sysroot`")?;
    if !out.status.success() {
        bail!("`rustc --print sysroot` failed");
    }
    let s = String::from_utf8(out.stdout)?.trim().to_string();
    Ok(PathBuf::from(s))
}

fn rustc_host() -> Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("running `rustc -vV`")?;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        if let Some(host) = line.strip_prefix("host:") {
            return Ok(host.trim().to_string());
        }
    }
    bail!("could not parse host from `rustc -vV`");
}

/// Check that `cargo-llvm-cov` is available; return the executable name.
pub fn require_cargo_llvm_cov() -> Result<&'static str> {
    let status = Command::new("cargo")
        .args(["llvm-cov", "--version"])
        .output();
    match status {
        Ok(o) if o.status.success() => Ok("cargo-llvm-cov"),
        _ => bail!(
            "`cargo-llvm-cov` is required but was not found.\n\
             hint: `cargo install cargo-llvm-cov` or use your distro's package"
        ),
    }
}

/// Run `cargo-llvm-cov show-env` in `dir` and parse the KEY=value lines into a
/// map. Values may be single-quoted; the surrounding quotes are stripped.
///
/// The returned environment must be supplied (on top of the inherited
/// environment) to any child `cargo`/test invocation that should produce
/// instrumented coverage.
///
/// `target_dir`, if given, is exported as `CARGO_TARGET_DIR` for the show-env
/// call (and should also be supplied to subsequent cargo/test runs). Using a
/// dedicated target dir isolates the instrumented build so it is never mixed
/// with — and silently reused from — a non-instrumented build.
pub fn coverage_env(dir: &Path, target_dir: Option<&str>) -> Result<BTreeMap<String, String>> {
    let mut cmd = Command::new("cargo");
    cmd.args(["llvm-cov", "show-env"]).current_dir(dir);
    if let Some(td) = target_dir {
        cmd.env("CARGO_TARGET_DIR", td);
    }
    let out = cmd.output().context("running `cargo llvm-cov show-env`")?;
    if !out.status.success() {
        bail!(
            "`cargo llvm-cov show-env` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut map = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with("info:") {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            &v[1..v.len() - 1]
        } else {
            v
        };
        map.insert(k.to_string(), v.to_string());
    }
    if !map.contains_key("LLVM_PROFILE_FILE") {
        bail!("show-env output did not contain LLVM_PROFILE_FILE");
    }
    Ok(map)
}

/// Locate the workspace root via `cargo metadata`.
pub fn workspace_root(dir: &Path) -> Result<PathBuf> {
    let out = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .current_dir(dir)
        .output()
        .context("running `cargo metadata`")?;
    if !out.status.success() {
        bail!(
            "`cargo metadata` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let root = v
        .get("workspace_root")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("metadata has no workspace_root"))?;
    Ok(PathBuf::from(root))
}
