use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Optional `.testmap.toml` at the project root. All fields are optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub collect: CollectConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectConfig {
    pub filter: Option<String>,
    pub skip: Option<String>,
    #[serde(default)]
    pub jobs: Option<usize>,
    #[serde(default)]
    pub threshold: Option<u32>,
}

impl Config {
    /// Load `.testmap.toml` from `dir` if present, else return defaults.
    pub fn load(dir: &Path) -> Result<Config> {
        let path = dir.join(".testmap.toml");
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }
}
