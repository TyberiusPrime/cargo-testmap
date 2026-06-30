pub mod coverage_run;
pub mod database;
pub mod lcov;
pub mod llvm;
pub mod test_discovery;

use crate::cli::CollectArgs;
use crate::config::Config;
use anyhow::{Context, Result};
use database::{Database, Metadata, ReverseMap, SourceFile, TestEntry};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use coverage_run::{run_single, TestCoverage};
use test_discovery::{list_tests, TestCase, TestTarget};

pub fn run(args: CollectArgs) -> Result<()> {
    let dir = PathBuf::from(".");
    let cfg = Config::load(&dir)?;

    // Merge config-file defaults with CLI flags (CLI wins).
    let filter = args.filter.or(cfg.collect.filter);
    let skip = args.skip.or(cfg.collect.skip);
    // `--threshold` defaults to 10, so we can't tell a user-provided 10 from
    // the default; treat 10 as "not set" and prefer the config-file value then.
    let threshold = match cfg.collect.threshold {
        Some(t) if args.threshold == 10 => t,
        _ => args.threshold,
    };
    let jobs = args.jobs.or(cfg.collect.jobs);

    let filter_re = match &filter {
        Some(p) => Some(Regex::new(p).with_context(|| format!("invalid --filter regex {p}"))?),
        None => None,
    };
    let skip_re = match &skip {
        Some(p) => Some(Regex::new(p).with_context(|| format!("invalid --skip regex {p}"))?),
        None => None,
    };

    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n.max(1))
            .build_global()
            .ok();
    }

    // --- Step 1: verify tooling & workspace --------------------------------
    llvm::require_cargo_llvm_cov()?;
    let tools = llvm::LlvmTools::discover()?;
    let workspace_root = llvm::workspace_root(&dir)?;
    let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);

    // --- Step 2: build all test binaries with instrumentation -----------
    // Use a dedicated target dir so the instrumented build is isolated from
    // the project's normal (non-instrumented) build artifacts.
    let output_path = PathBuf::from(&args.output);
    let testmap_root = output_path
        .parent()
        .unwrap_or_else(|| Path::new("target/testmap"))
        .to_path_buf();
    let cov_target_dir = testmap_root.join("cov-target");
    let cov_target_dir_str = cov_target_dir.to_string_lossy().into_owned();
    if args.clean {
        let _ = std::fs::remove_dir_all(&cov_target_dir);
    }
    // Assemble the cargo target-selection arguments (forwarded to the build).
    let mut cargo_select: Vec<String> = Vec::new();
    if args.workspace {
        cargo_select.push("--workspace".to_string());
    }
    if let Some(p) = &args.package {
        cargo_select.push("--package".to_string());
        cargo_select.push(p.clone());
    }
    for (flag, set) in [
        ("--lib", args.lib),
        ("--bins", args.bins),
        ("--tests", args.tests),
    ] {
        if set {
            cargo_select.push(flag.to_string());
        }
    }
    cargo_select.extend(args.cargo_args.iter().cloned());
    eprintln!("→ building instrumented test binaries…");
    let built_targets = test_discovery::build_targets(
        &dir,
        &cargo_select,
        &cov_target_dir_str,
        args.verbose,
    )?;

    // --- Step 3: enumerate tests per binary -------------------------------
    let mut cases: Vec<(TestCase, TestTarget)> = Vec::new();
    for (idx, target) in built_targets.iter().enumerate() {
        let mut found = list_tests(target)
            .with_context(|| format!("listing tests for target `{}`", target.name))?;
        for c in &mut found {
            c.target_index = idx;
        }
        eprintln!("  {}: {} test(s)", target.name, found.len());
        for c in found {
            cases.push((c, target.clone()));
        }
    }

    // Apply --filter / --skip.
    let mut selected: Vec<(TestCase, TestTarget)> = Vec::new();
    for (c, t) in cases {
        if let Some(re) = &filter_re
            && !re.is_match(&c.full) {
                continue;
            }
        if let Some(re) = &skip_re
            && re.is_match(&c.full) {
                continue;
            }
        selected.push((c, t));
    }
    if selected.is_empty() {
        anyhow::bail!("no tests matched the current filters");
    }
    eprintln!("→ {} test(s) to collect", selected.len());

    // --- Step 4: run each test in parallel, accumulating coverage ---------
    let staging = testmap_root.join("staging");
    if args.clean {
        let _ = std::fs::remove_dir_all(&staging);
    }
    std::fs::create_dir_all(&staging)?;

    let bar = ProgressBar::new(selected.len() as u64);
    bar.set_style(
        ProgressStyle::with_template(
            "{elapsed_precise} [{bar:30.cyan/blue}] {pos}/{len} ({per_sec}) {msg}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    // Stable, deterministic order so test indices are reproducible regardless
    // of scheduling. The position in this vector *is* the test index used in
    // the database.
    let mut indexed: Vec<(TestCase, TestTarget)> = selected.into_iter().collect();
    indexed.sort_by(|a, b| a.0.full.cmp(&b.0.full).then(a.0.target_index.cmp(&b.0.target_index)));

    let results: Vec<Option<(usize, TestCoverage)>> = indexed
        .par_iter()
        .enumerate()
        .map(|(i, (case, target))| {
            let r = run_single(
                &target.executable,
                &target.name,
                &case.full,
                &tools,
                &staging,
                &workspace_root,
                args.clean,
            );
            bar.inc(1);
            match r {
                Ok(cov) => Some((i, cov)),
                Err(e) => {
                    bar.println(format!("warning: {}: {e}", case.full));
                    None
                }
            }
        })
        .collect();
    bar.finish_and_clear();

    // --- Step 5: build the test table & reverse map -----------------------
    let mut tests: Vec<TestEntry> = indexed
        .iter()
        .map(|(case, target)| TestEntry {
            name: case.name.clone(),
            module: case.module.clone(),
            binary: target.name.clone(),
            kind: target.kind.clone(),
            status: "collected".to_string(), // refined below
            duration_ms: 0,
        })
        .collect();
    let mut map = ReverseMap::new();
    let mut failures = 0u64;
    for r in results {
        let Some((idx, cov)) = r else { continue };
        if matches!(cov.status, coverage_run::TestStatus::Failed) {
            failures += 1;
        }
        tests[idx].status = cov.status.as_str().to_string();
        tests[idx].duration_ms = cov.duration_ms;
        for (rel, lines) in cov.records {
            for line in lines {
                map.record(&rel, line, idx as u32);
            }
        }
    }

    // --- Step 6: apply threshold + snapshot sources, write DB -------------
    eprintln!("→ writing database (threshold = {threshold})…");
    let sources: BTreeMap<String, SourceFile> = map.finalize(threshold, &workspace_root)?;

    let db = Database {
        version: 1,
        metadata: Metadata {
            generated_at: crate::util::now_rfc3339(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
            cargo_testmap_version: env!("CARGO_PKG_VERSION").to_string(),
            collection_args: cargo_select.clone(),
        },
        tests,
        sources,
    };

    let out = PathBuf::from(&args.output);
    db.write(&out)?;
    let n_files = db.sources.len();
    let n_lines: usize = db.sources.values().map(|s| s.lines.len()).sum();
    eprintln!(
        "✓ wrote {} ({} test(s){}, {} source file(s), {} mapped line(s))",
        out.display(),
        db.tests.len(),
        if failures > 0 {
            format!(", {failures} failed")
        } else {
            String::new()
        },
        n_files,
        n_lines
    );
    Ok(())
}
