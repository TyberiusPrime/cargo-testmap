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
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SourceFile {
    pub content: String,
    /// line number (1-based, as string) -> sorted test indices.
    pub lines: BTreeMap<String, Vec<u32>>,
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
            for (line, tests) in lines {
                // Omit lines covered by >= threshold tests (see DESIGN §3.3).
                if (tests.len() as u32) >= threshold {
                    continue;
                }
                let mut v: Vec<u32> = tests.into_iter().collect();
                v.sort_unstable();
                filtered.insert(line.to_string(), v);
            }
            if filtered.is_empty() {
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
                },
            );
        }
        Ok(out)
    }
}
