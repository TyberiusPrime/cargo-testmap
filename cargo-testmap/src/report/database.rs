use serde::Deserialize;
use std::collections::BTreeMap;

/// The deserialized testmap database. Mirrors the collect-side schema.
#[derive(Deserialize)]
pub struct Database {
    pub version: u32,
    #[allow(dead_code)]
    pub metadata: Metadata,
    pub tests: Vec<TestEntry>,
    pub sources: BTreeMap<String, SourceFile>,
}

#[derive(Deserialize)]
pub struct Metadata {
    #[allow(dead_code)]
    pub generated_at: String,
    #[allow(dead_code)]
    pub workspace_root: String,
    #[allow(dead_code)]
    pub cargo_testmap_version: String,
    #[allow(dead_code)]
    pub collection_args: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub struct TestEntry {
    pub name: String,
    pub module: String,
    pub binary: String,
    pub kind: String,
    pub status: String,
    #[allow(dead_code)]
    pub duration_ms: u64,
    /// Captured output of a failed test; shown in the tests catalog.
    #[serde(default)]
    pub failure_output: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct SourceFile {
    pub content: String,
    pub lines: BTreeMap<String, Vec<u32>>,
    /// line -> info for lines covered by >= threshold tests (a sample of the
    /// covering tests plus the total count).  Shown with a dot and an
    /// "above threshold" note.
    #[serde(default)]
    pub above_threshold: BTreeMap<String, AboveThreshold>,
    /// Every executable (instrumented) line in the file, covered or not.
    /// `#[serde(default)]` so databases written before this field existed
    /// still load (they'll simply report no coverage gaps).
    #[serde(default)]
    pub executable: Vec<u32>,
}

#[derive(Deserialize, Clone)]
pub struct AboveThreshold {
    pub total: u32,
    pub sample: Vec<u32>,
}

impl Database {
    pub fn read(path: &std::path::Path) -> anyhow::Result<Database> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading database {}: {e}", path.display()))?;
        let db: Database = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing database {}: {e}", path.display()))?;
        if db.version != 1 {
            anyhow::bail!(
                "unsupported database version {} (this build of cargo-testmap expects 1)",
                db.version
            );
        }
        Ok(db)
    }
}
