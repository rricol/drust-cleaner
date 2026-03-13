use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Top-level configuration file structure.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Optional global settings.
    #[serde(default)]
    pub settings: Settings,

    /// Ordered list of rules. Rules are evaluated top-to-bottom; the first
    /// matching rule wins.
    pub rules: Vec<Rule>,
}

/// Global settings that apply to the whole run.
#[derive(Debug, Deserialize)]
pub struct Settings {
    /// When true, recurse into sub-directories when scanning for files.
    #[serde(default)]
    pub recursive: bool,

    /// If set, files that don't match any rule are moved into this folder.
    pub unmatched_destination: Option<String>,

    /// Global ignore list: exact filenames or glob patterns matched against
    /// the filename only.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// When true (default), files whose names start with '.' are skipped.
    #[serde(default = "default_true")]
    pub ignore_hidden: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recursive: false,
            unmatched_destination: None,
            ignore: Vec::new(),
            ignore_hidden: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// A single organisation rule.
#[derive(Debug, Deserialize)]
pub struct Rule {
    /// Human-readable label used in log output.
    pub name: String,

    /// Destination folder path, relative to the target directory.
    pub destination: String,

    /// List of file extensions to match (no leading dot, case-insensitive).
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Glob pattern matched against the file name only.
    pub name_pattern: Option<String>,

    /// Minimum file size in megabytes (inclusive).
    pub min_size_mb: Option<f64>,

    /// Maximum file size in megabytes (inclusive).
    pub max_size_mb: Option<f64>,

    /// Per-rule ignore list.
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl Rule {
    pub fn has_conditions(&self) -> bool {
        !self.extensions.is_empty()
            || self.name_pattern.is_some()
            || self.min_size_mb.is_some()
            || self.max_size_mb.is_some()
    }
}

/// Load and parse a TOML config file from `path`.
pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    let raw = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read config file '{}': {}", path.display(), e))?;

    let config: Config = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse config file '{}': {}", path.display(), e))?;

    validate_config(&config)?;

    Ok(config)
}

fn validate_config(config: &Config) -> anyhow::Result<()> {
    if config.rules.is_empty() {
        anyhow::bail!("Config file contains no rules.");
    }

    for (i, rule) in config.rules.iter().enumerate() {
        if rule.name.trim().is_empty() {
            anyhow::bail!("Rule #{} has an empty 'name' field.", i + 1);
        }
        if rule.destination.trim().is_empty() {
            anyhow::bail!("Rule '{}' has an empty 'destination' field.", rule.name);
        }
        if !rule.has_conditions() {
            anyhow::bail!(
                "Rule '{}' has no conditions (extensions, name_pattern, min_size_mb, \
                 max_size_mb). This would match every file.",
                rule.name
            );
        }
        if let (Some(min), Some(max)) = (rule.min_size_mb, rule.max_size_mb) {
            if min > max {
                anyhow::bail!(
                    "Rule '{}': min_size_mb ({}) is greater than max_size_mb ({}).",
                    rule.name,
                    min,
                    max
                );
            }
        }
    }

    Ok(())
}
