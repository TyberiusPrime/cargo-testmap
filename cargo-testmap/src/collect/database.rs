use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The serialized testmap database (testmap.json). See DESIGN §4.
#[derive(Serialize, Deserialize)]
pub struct Database {
    pub version: u32,
    pub metadata: Metadata,
    pub tests: Vec<TestEntry>,
    pub sources: BTreeMap<String, SourceFile>,
}

#[derive(Serialize, Deserialize)]
pub struct Metadata {
    pub generated_at: String,
    pub workspace_root: String,
    pub cargo_testmap_version: String,
    pub collection_args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TestEntry {
    pub name: String,
    pub module: String,
    pub binary: String,
    pub kind: String,
    pub status: String,
    pub duration_ms: u64,
    /// Captured output (stderr+stdout) of a test that ran but failed.
    /// `None` for passing tests. Persisted so the report can surface *why*
    /// a test failed instead of just tallying the count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_output: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SourceFile {
    pub content: String,
    /// line number (1-based, as string) -> sorted test indices, for lines
    /// covered by fewer than `threshold` tests.
    pub lines: BTreeMap<String, Vec<u32>>,
    /// line number -> info for lines covered by *at least* `threshold` tests.
    /// We can't list every covering test (too many), so we keep the total count
    /// plus a small representative sample.  The report renders the sample with
    /// an "above threshold" note.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub above_threshold: BTreeMap<String, AboveThreshold>,
}

/// Coverage summary for a line hit by >= threshold tests.
#[derive(Serialize, Deserialize, Clone)]
pub struct AboveThreshold {
    /// Total number of tests covering the line.
    pub total: u32,
    /// A deterministic random sample of covering test indices, of size
    /// `threshold - 1`.  Deterministic (seeded by path+line) so the database
    /// stays reproducible across runs.
    pub sample: Vec<u32>,
}

impl Database {
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

/// In-memory reverse map accumulated before writing the database.
///
/// Key: relative source path. Value: per-line set of test indices.
pub struct ReverseMap(BTreeMap<String, BTreeMap<u32, BTreeSet<u32>>>);

use std::collections::BTreeSet;
impl ReverseMap {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn record(&mut self, rel_path: &str, line: u32, test_index: u32) {
        self.0
            .entry(rel_path.to_string())
            .or_default()
            .entry(line)
            .or_default()
            .insert(test_index);
    }

    /// Apply the threshold filter and snapshot source file contents.
    pub fn finalize(
        self,
        threshold: u32,
        workspace_root: &Path,
    ) -> anyhow::Result<BTreeMap<String, SourceFile>> {
        let mut out = BTreeMap::new();
        for (rel_path, lines) in self.0 {
            let mut filtered: BTreeMap<String, Vec<u32>> = BTreeMap::new();
            let mut above: BTreeMap<String, AboveThreshold> = BTreeMap::new();
            for (line, tests) in lines {
                let count = tests.len() as u32;
                if count >= threshold {
                    // Too many tests to list.  Keep a deterministic random
                    // sample (size threshold-1) plus the total, so the report
                    // can show *something* useful with an "above threshold" note.
                    let mut all: Vec<u32> = tests.into_iter().collect();
                    all.sort_unstable();
                    let want = threshold.saturating_sub(1) as usize;
                    let seed = crate::util::fnv1a_u64(&format!("{rel_path}:{line}"));
                    let sample = sample_sorted(&all, want, seed);
                    above.insert(
                        line.to_string(),
                        AboveThreshold { total: count, sample },
                    );
                } else {
                    let mut v: Vec<u32> = tests.into_iter().collect();
                    v.sort_unstable();
                    filtered.insert(line.to_string(), v);
                }
            }
            if filtered.is_empty() && above.is_empty() {
                // No interesting lines for this file — skip it entirely.
                continue;
            }
            let abs = workspace_root.join(&rel_path);
            let content = match std::fs::read_to_string(&abs) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "warning: could not read {} for snapshot: {e}",
                        abs.display()
                    );
                    continue;
                }
            };
            out.insert(
                rel_path,
                SourceFile {
                    content,
                    lines: filtered,
                    above_threshold: above,
                },
            );
        }
        Ok(out)
    }
}

/// Pick `want` elements from `all` in a deterministic-but-random-looking
/// order (xorshift64 Fisher–Yates seeded by `seed`), returned sorted for a
/// stable display.  If `want >= all.len()`, every element is returned.  Used
/// to keep a small sample of covering tests for above-threshold lines.
fn sample_sorted(all: &[u32], want: usize, seed: u64) -> Vec<u32> {
    if want == 0 {
        return Vec::new();
    }
    if want >= all.len() {
        let mut v = all.to_vec();
        v.sort_unstable();
        return v;
    }
    let mut shuffled: Vec<u32> = all.to_vec();
    // xorshift64 — good enough for UI sampling, and fully deterministic.
    let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    if state == 0 {
        state = 0xDEAD_BEEF_DEAD_BEEF;
    }
    for i in (1..shuffled.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        shuffled.swap(i, j);
    }
    let mut picked = shuffled[..want].to_vec();
    picked.sort_unstable();
    picked
}
