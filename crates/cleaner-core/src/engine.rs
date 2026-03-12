use std::fs;
use std::path::{Path, PathBuf};

use glob::Pattern;
use walkdir::WalkDir;

use crate::config::{Config, Rule, Settings};

fn is_ignored(file_name: &str, ignore_list: &[String]) -> bool {
    ignore_list.iter().any(|pattern| {
        match glob::Pattern::new(pattern) {
            Ok(p) => p.matches(file_name),
            Err(_) => file_name == pattern,
        }
    })
}

const MB: f64 = 1024.0 * 1024.0;

/// Summary of a completed (or simulated) run.
#[derive(Debug, Default)]
pub struct RunSummary {
    pub moved: usize,
    pub skipped: usize,
    pub errors: usize,
    /// Log messages produced during the run (plain text, no ANSI codes).
    pub messages: Vec<String>,
}

/// A single planned file operation.
#[derive(Debug)]
pub struct FileAction {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub rule_name: String,
}

/// Run the engine against `target_dir` using the given `config`.
///
/// - `dry_run = true`  → collect planned actions as messages, move nothing.
/// - `dry_run = false` → execute moves, collect results as messages.
pub fn run(
    target_dir: &Path,
    config: &Config,
    config_file_name: &str,
    dry_run: bool,
) -> anyhow::Result<RunSummary> {
    let actions = collect_actions(target_dir, config, config_file_name)?;

    let mut summary = RunSummary::default();

    if actions.is_empty() {
        summary.messages.push("No files matched any rule.".to_string());
        return Ok(summary);
    }

    for action in &actions {
        if dry_run {
            summary.messages.push(format!(
                "[DRY-RUN] {} → {}  (rule: {})",
                action.source.display(),
                action.destination.display(),
                action.rule_name,
            ));
            summary.moved += 1;
        } else {
            match execute_move(action) {
                Ok(()) => {
                    summary.messages.push(format!(
                        "MOVED {} → {}  (rule: {})",
                        action.source.display(),
                        action.destination.display(),
                        action.rule_name,
                    ));
                    summary.moved += 1;
                }
                Err(e) => {
                    summary.messages.push(format!(
                        "ERROR {}: {}",
                        action.source.display(),
                        e,
                    ));
                    summary.errors += 1;
                }
            }
        }
    }

    // Handle unmatched files
    if config.settings.unmatched_destination.is_some() && !dry_run {
        let matched_sources: std::collections::HashSet<&PathBuf> =
            actions.iter().map(|a| &a.source).collect();

        let unmatched = collect_files(target_dir, &config.settings)
            .into_iter()
            .filter(|p| {
                if matched_sources.contains(p) {
                    return false;
                }
                let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                !is_ignored(file_name, &config.settings.ignore) && file_name != config_file_name
            })
            .collect::<Vec<_>>();

        let dest_name = config.settings.unmatched_destination.as_deref().unwrap();
        for file in unmatched {
            let dest = build_destination(target_dir, &file, dest_name)?;
            let action = FileAction {
                source: file,
                destination: dest,
                rule_name: "<unmatched>".to_string(),
            };
            match execute_move(&action) {
                Ok(()) => {
                    summary.messages.push(format!(
                        "MOVED (unmatched) {} → {}",
                        action.source.display(),
                        action.destination.display(),
                    ));
                    summary.moved += 1;
                }
                Err(e) => {
                    summary.messages.push(format!(
                        "ERROR {}: {}",
                        action.source.display(),
                        e,
                    ));
                    summary.errors += 1;
                }
            }
        }
    }

    Ok(summary)
}

fn collect_actions(
    target_dir: &Path,
    config: &Config,
    config_file_name: &str,
) -> anyhow::Result<Vec<FileAction>> {
    let files = collect_files(target_dir, &config.settings);
    let mut actions: Vec<FileAction> = Vec::new();

    'file: for file_path in files {
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if file_name == config_file_name {
            continue;
        }

        if is_ignored(file_name, &config.settings.ignore) {
            continue;
        }

        let metadata = match fs::metadata(&file_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for rule in &config.rules {
            if matches_rule(rule, &file_path, &metadata) {
                let destination = build_destination(target_dir, &file_path, &rule.destination)?;
                if file_path.parent() == destination.parent() {
                    continue 'file;
                }
                actions.push(FileAction {
                    source: file_path,
                    destination,
                    rule_name: rule.name.clone(),
                });
                continue 'file;
            }
        }
    }

    Ok(actions)
}

fn collect_files(target_dir: &Path, settings: &Settings) -> Vec<PathBuf> {
    let depth = if settings.recursive { usize::MAX } else { 1 };

    WalkDir::new(target_dir)
        .max_depth(depth)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect()
}

fn matches_rule(rule: &Rule, file_path: &Path, metadata: &fs::Metadata) -> bool {
    let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if is_ignored(file_name, &rule.ignore) {
        return false;
    }

    if !rule.extensions.is_empty() {
        let file_ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !rule.extensions.iter().any(|ext| ext.to_lowercase() == file_ext) {
            return false;
        }
    }

    if let Some(ref pattern_str) = rule.name_pattern {
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        match Pattern::new(pattern_str) {
            Ok(pattern) => {
                if !pattern.matches(file_name) {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }

    let file_size_mb = metadata.len() as f64 / MB;
    if let Some(min) = rule.min_size_mb {
        if file_size_mb < min {
            return false;
        }
    }
    if let Some(max) = rule.max_size_mb {
        if file_size_mb > max {
            return false;
        }
    }

    true
}

fn build_destination(
    target_dir: &Path,
    file_path: &Path,
    destination_folder: &str,
) -> anyhow::Result<PathBuf> {
    let file_name = file_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("File has no name: {}", file_path.display()))?;
    Ok(target_dir.join(destination_folder).join(file_name))
}

fn execute_move(action: &FileAction) -> anyhow::Result<()> {
    let dest_dir = action
        .destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Destination has no parent directory"))?;

    fs::create_dir_all(dest_dir)
        .map_err(|e| anyhow::anyhow!("Could not create directory '{}': {}", dest_dir.display(), e))?;

    if fs::rename(&action.source, &action.destination).is_err() {
        fs::copy(&action.source, &action.destination).map_err(|e| {
            anyhow::anyhow!(
                "Failed to copy '{}' to '{}': {}",
                action.source.display(),
                action.destination.display(),
                e
            )
        })?;
        fs::remove_file(&action.source).map_err(|e| {
            anyhow::anyhow!(
                "Copied file but failed to remove source '{}': {}",
                action.source.display(),
                e
            )
        })?;
    }

    Ok(())
}
