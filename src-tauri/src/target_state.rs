use crate::adapters::{EntryType, FileEntry};
use crate::path_utils;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const MAX_RUN_HISTORY: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetIdentity {
    pub normalized_url: String,
    pub host: String,
    pub logical_id: Option<String>,
    pub target_key: String,
}

#[derive(Clone, Debug)]
pub struct TargetPaths {
    pub target_identity: TargetIdentity,
    pub support_root: PathBuf,
    pub target_dir: PathBuf,
    pub history_dir: PathBuf,
    pub ledger_path: PathBuf,
    pub current_snapshot_path: PathBuf,
    pub best_snapshot_path: PathBuf,
    pub failure_manifest_path: PathBuf,
    pub latest_resume_plan_path: PathBuf,
    pub stable_current_listing_path: PathBuf,
    pub stable_current_dirs_listing_path: PathBuf,
    pub stable_best_listing_path: PathBuf,
    pub stable_best_dirs_listing_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CrawlOutcome {
    FirstRun,
    MatchedBest,
    ExceededBest,
    Degraded,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrawlRunRecord {
    pub started_at_epoch: u64,
    pub finished_at_epoch: u64,
    pub raw_this_run_count: usize,
    pub best_prior_count: usize,
    pub merged_effective_count: usize,
    pub outcome: String,
    pub retry_count_used: usize,
    pub instability_reasons: Vec<String>,
    pub current_listing_path: String,
    pub current_dirs_listing_path: String,
    pub history_canonical_path: String,
    pub history_dirs_path: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetLedger {
    pub target_key: String,
    pub normalized_url: String,
    pub host: String,
    pub logical_id: Option<String>,
    pub stable_current_listing_path: String,
    pub stable_current_dirs_listing_path: String,
    pub stable_best_listing_path: String,
    pub stable_best_dirs_listing_path: String,
    pub current_snapshot_path: String,
    pub best_snapshot_path: String,
    pub history_dir: String,
    pub failure_manifest_path: String,
    pub latest_resume_plan_path: String,
    pub best_known_count: usize,
    pub best_snapshot_version: usize,
    pub last_outcome: Option<String>,
    pub runs: Vec<CrawlRunRecord>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFailureRecord {
    pub path: String,
    pub source_url: String,
    pub size_hint: Option<u64>,
    pub last_error: String,
    pub stage: String,
    pub attempt_count: usize,
    pub last_attempt_epoch: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResumePlan {
    pub target_key: String,
    pub failed_first_count: usize,
    pub missing_or_mismatch_count: usize,
    pub skipped_exact_matches_count: usize,
    pub all_items_skipped: bool,
    pub planned_file_count: usize,
    pub failure_manifest_path: String,
}

#[derive(Clone, Debug)]
pub struct DownloadResumePlanBuild {
    pub plan: DownloadResumePlan,
    pub ordered_entries: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub struct ListingArtifactPaths {
    pub current_canonical_path: PathBuf,
    pub current_dirs_path: PathBuf,
    pub history_canonical_path: PathBuf,
    pub history_dirs_path: PathBuf,
}

pub fn derive_target_identity(url: &str) -> TargetIdentity {
    let normalized_url = normalized_target_url(url);
    let parsed = reqwest::Url::parse(&normalized_url).ok();
    let host = parsed
        .as_ref()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        .unwrap_or_else(|| extract_host_fallback(&normalized_url));
    let logical_id = parsed
        .as_ref()
        .and_then(logical_id_from_parsed_url)
        .or_else(|| {
            let fallback = path_utils::extract_target_dirname(url);
            if fallback.is_empty() {
                None
            } else {
                Some(fallback)
            }
        });

    let host_component = sanitize_key_component(host.trim_end_matches(".onion"), 28);
    let logical_component = sanitize_key_component(logical_id.as_deref().unwrap_or("root"), 24);
    let hash_component = short_hash(&normalized_url);
    let target_key = format!("{host_component}__{logical_component}__{hash_component}");

    TargetIdentity {
        normalized_url,
        host,
        logical_id,
        target_key,
    }
}

pub fn target_paths(output_root: &Path, target_url: &str) -> Result<TargetPaths> {
    let identity = derive_target_identity(target_url);
    let support_root = output_root.join("temp_onionforge_forger");
    let target_dir = support_root.join("targets").join(&identity.target_key);
    let history_dir = target_dir.join("crawl_history");
    std::fs::create_dir_all(&history_dir)?;

    Ok(TargetPaths {
        target_identity: identity.clone(),
        support_root,
        target_dir: target_dir.clone(),
        history_dir,
        ledger_path: target_dir.join("target_ledger.json"),
        current_snapshot_path: target_dir.join("crawl_current_entries.json"),
        best_snapshot_path: target_dir.join("crawl_best_entries.json"),
        failure_manifest_path: target_dir.join("download_failures.json"),
        latest_resume_plan_path: target_dir.join("download_resume_plan.json"),
        stable_current_listing_path: output_root
            .join(format!("{}__crawl_current.txt", identity.target_key)),
        stable_current_dirs_listing_path: output_root
            .join(format!("{}__crawl_current_dirs.txt", identity.target_key)),
        stable_best_listing_path: output_root
            .join(format!("{}__crawl_best.txt", identity.target_key)),
        stable_best_dirs_listing_path: output_root
            .join(format!("{}__crawl_best_dirs.txt", identity.target_key)),
    })
}

pub fn load_or_default_ledger(paths: &TargetPaths) -> Result<TargetLedger> {
    if paths.ledger_path.exists() {
        let data = std::fs::read(&paths.ledger_path)
            .with_context(|| format!("read ledger at {}", paths.ledger_path.display()))?;
        let ledger = serde_json::from_slice::<TargetLedger>(&data)
            .with_context(|| format!("parse ledger at {}", paths.ledger_path.display()))?;
        return Ok(ledger);
    }

    Ok(TargetLedger {
        target_key: paths.target_identity.target_key.clone(),
        normalized_url: paths.target_identity.normalized_url.clone(),
        host: paths.target_identity.host.clone(),
        logical_id: paths.target_identity.logical_id.clone(),
        stable_current_listing_path: paths
            .stable_current_listing_path
            .to_string_lossy()
            .to_string(),
        stable_current_dirs_listing_path: paths
            .stable_current_dirs_listing_path
            .to_string_lossy()
            .to_string(),
        stable_best_listing_path: paths.stable_best_listing_path.to_string_lossy().to_string(),
        stable_best_dirs_listing_path: paths
            .stable_best_dirs_listing_path
            .to_string_lossy()
            .to_string(),
        current_snapshot_path: paths.current_snapshot_path.to_string_lossy().to_string(),
        best_snapshot_path: paths.best_snapshot_path.to_string_lossy().to_string(),
        history_dir: paths.history_dir.to_string_lossy().to_string(),
        failure_manifest_path: paths.failure_manifest_path.to_string_lossy().to_string(),
        latest_resume_plan_path: paths.latest_resume_plan_path.to_string_lossy().to_string(),
        best_known_count: 0,
        best_snapshot_version: 1,
        last_outcome: None,
        runs: Vec::new(),
    })
}

pub fn save_ledger(paths: &TargetPaths, ledger: &TargetLedger) -> Result<()> {
    let data = serde_json::to_vec_pretty(ledger)?;
    std::fs::write(&paths.ledger_path, data)
        .with_context(|| format!("write ledger to {}", paths.ledger_path.display()))?;
    Ok(())
}

pub fn save_entries_snapshot(path: &Path, entries: &[FileEntry]) -> Result<()> {
    let data = serde_json::to_vec_pretty(entries)?;
    std::fs::write(path, data).with_context(|| format!("write snapshot to {}", path.display()))?;
    Ok(())
}

pub fn load_entries_snapshot(path: &Path) -> Result<Vec<FileEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read(path).with_context(|| format!("read snapshot {}", path.display()))?;
    let entries = serde_json::from_slice::<Vec<FileEntry>>(&data)
        .with_context(|| format!("parse snapshot {}", path.display()))?;
    Ok(entries)
}

pub fn load_failure_manifest(path: &Path) -> Result<Vec<DownloadFailureRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read(path).with_context(|| format!("read failures {}", path.display()))?;
    let records = serde_json::from_slice::<Vec<DownloadFailureRecord>>(&data)
        .with_context(|| format!("parse failures {}", path.display()))?;
    Ok(records)
}

pub fn save_failure_manifest(path: &Path, records: &[DownloadFailureRecord]) -> Result<()> {
    let data = serde_json::to_vec_pretty(records)?;
    std::fs::write(path, data).with_context(|| format!("write failures to {}", path.display()))?;
    Ok(())
}

pub fn save_resume_plan(path: &Path, plan: &DownloadResumePlan) -> Result<()> {
    let data = serde_json::to_vec_pretty(plan)?;
    std::fs::write(path, data)
        .with_context(|| format!("write resume plan to {}", path.display()))?;
    Ok(())
}

pub fn merge_entries(
    best_prior_entries: &[FileEntry],
    current_entries: &[FileEntry],
) -> Vec<FileEntry> {
    let mut merged = BTreeMap::<String, FileEntry>::new();
    for entry in best_prior_entries {
        merged.insert(entry.path.clone(), entry.clone());
    }
    for entry in current_entries {
        match merged.get_mut(&entry.path) {
            Some(existing) => {
                if should_replace_entry(existing, entry) {
                    *existing = entry.clone();
                }
            }
            None => {
                merged.insert(entry.path.clone(), entry.clone());
            }
        }
    }
    merged.into_values().collect()
}

pub fn write_current_and_history_listings(
    paths: &TargetPaths,
    entries: &[FileEntry],
    target_url: &str,
) -> Result<ListingArtifactPaths> {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let history_canonical_path = paths
        .history_dir
        .join(format!("{}__canonical.txt", timestamp));
    let history_dirs_path = paths.history_dir.join(format!("{}__dirs.txt", timestamp));
    let canonical =
        render_canonical_listing(target_url, &paths.target_identity.target_key, entries);
    let dirs = render_dir_listing(target_url, &paths.target_identity.target_key, entries);

    std::fs::write(&paths.stable_current_listing_path, canonical.as_bytes())?;
    std::fs::write(&paths.stable_current_dirs_listing_path, dirs.as_bytes())?;
    std::fs::write(&history_canonical_path, canonical.as_bytes())?;
    std::fs::write(&history_dirs_path, dirs.as_bytes())?;

    Ok(ListingArtifactPaths {
        current_canonical_path: paths.stable_current_listing_path.clone(),
        current_dirs_path: paths.stable_current_dirs_listing_path.clone(),
        history_canonical_path,
        history_dirs_path,
    })
}

pub fn write_best_listings(
    paths: &TargetPaths,
    entries: &[FileEntry],
    target_url: &str,
) -> Result<()> {
    let canonical =
        render_canonical_listing(target_url, &paths.target_identity.target_key, entries);
    let dirs = render_dir_listing(target_url, &paths.target_identity.target_key, entries);
    std::fs::write(&paths.stable_best_listing_path, canonical.as_bytes())?;
    std::fs::write(&paths.stable_best_dirs_listing_path, dirs.as_bytes())?;
    Ok(())
}

pub fn append_run_record(
    ledger: &mut TargetLedger,
    run_record: CrawlRunRecord,
    outcome: CrawlOutcome,
    best_known_count: usize,
) {
    ledger.best_known_count = best_known_count;
    ledger.last_outcome = Some(crawl_outcome_label(outcome).to_string());
    ledger.runs.push(run_record);
    if ledger.runs.len() > MAX_RUN_HISTORY {
        let overflow = ledger.runs.len() - MAX_RUN_HISTORY;
        ledger.runs.drain(0..overflow);
    }
}

pub fn crawl_outcome_label(outcome: CrawlOutcome) -> &'static str {
    match outcome {
        CrawlOutcome::FirstRun => "first_run",
        CrawlOutcome::MatchedBest => "matched_best",
        CrawlOutcome::ExceededBest => "exceeded_best",
        CrawlOutcome::Degraded => "degraded",
    }
}

pub fn build_download_resume_plan(
    target_key: &str,
    best_entries: &[FileEntry],
    failure_records: &[DownloadFailureRecord],
    output_root: &Path,
    failure_manifest_path: &Path,
) -> Result<DownloadResumePlanBuild> {
    let best_files: HashMap<String, FileEntry> = best_entries
        .iter()
        .filter(|entry| entry.entry_type == EntryType::File)
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect();

    let mut ordered_entries = Vec::new();
    let mut queued_paths = HashSet::new();
    let mut failed_first_count = 0usize;
    let mut missing_or_mismatch_count = 0usize;
    let mut skipped_exact_matches_count = 0usize;

    for record in failure_records {
        if let Some(entry) = best_files.get(&record.path) {
            match file_download_state(output_root, entry)? {
                FileDownloadState::NeedsDownload => {
                    if queued_paths.insert(entry.path.clone()) {
                        ordered_entries.push(entry.clone());
                        failed_first_count += 1;
                    }
                }
                FileDownloadState::ExactMatch => {
                    skipped_exact_matches_count += 1;
                }
                FileDownloadState::PresentWithoutSize => {}
            }
        }
    }

    let mut remaining_entries: Vec<FileEntry> = best_files
        .values()
        .filter(|entry| !queued_paths.contains(&entry.path))
        .cloned()
        .collect();
    remaining_entries.sort_by(|a, b| a.path.cmp(&b.path));

    for entry in remaining_entries {
        match file_download_state(output_root, &entry)? {
            FileDownloadState::NeedsDownload => {
                ordered_entries.push(entry);
                missing_or_mismatch_count += 1;
            }
            FileDownloadState::ExactMatch => {
                skipped_exact_matches_count += 1;
            }
            FileDownloadState::PresentWithoutSize => {}
        }
    }

    let plan = DownloadResumePlan {
        target_key: target_key.to_string(),
        failed_first_count,
        missing_or_mismatch_count,
        skipped_exact_matches_count,
        all_items_skipped: ordered_entries.is_empty(),
        planned_file_count: ordered_entries.len(),
        failure_manifest_path: failure_manifest_path.to_string_lossy().to_string(),
    };

    Ok(DownloadResumePlanBuild {
        plan,
        ordered_entries,
    })
}

pub fn reconcile_failure_manifest(
    previous_records: &[DownloadFailureRecord],
    planned_entries: &[FileEntry],
    authoritative_entries: &[FileEntry],
    output_root: &Path,
    default_stage: &str,
) -> Result<Vec<DownloadFailureRecord>> {
    let previous_map: HashMap<String, DownloadFailureRecord> = previous_records
        .iter()
        .cloned()
        .map(|record| (record.path.clone(), record))
        .collect();
    let authoritative_paths: HashSet<&str> = authoritative_entries
        .iter()
        .filter(|entry| entry.entry_type == EntryType::File)
        .map(|entry| entry.path.as_str())
        .collect();

    let mut next_records = Vec::new();
    let now = unix_now();

    for entry in authoritative_entries
        .iter()
        .filter(|entry| entry.entry_type == EntryType::File)
    {
        let state = file_download_state(output_root, entry)?;
        if matches!(state, FileDownloadState::NeedsDownload) {
            let previous = previous_map.get(&entry.path);
            let was_planned = planned_entries
                .iter()
                .any(|planned| planned.path == entry.path);
            let mut record = previous.cloned().unwrap_or_else(|| DownloadFailureRecord {
                path: entry.path.clone(),
                source_url: entry.raw_url.clone(),
                size_hint: entry.size_bytes,
                last_error: "missing_or_size_mismatch".to_string(),
                stage: default_stage.to_string(),
                attempt_count: 0,
                last_attempt_epoch: 0,
            });
            if was_planned {
                record.attempt_count = record.attempt_count.saturating_add(1).max(1);
                record.last_attempt_epoch = now;
                record.stage = default_stage.to_string();
                record.last_error = if entry.size_bytes.unwrap_or(0) > 0 {
                    "size_mismatch_or_missing_after_resume".to_string()
                } else {
                    "missing_after_resume".to_string()
                };
            }
            next_records.push(record);
        }
    }

    for previous in previous_records {
        if !authoritative_paths.contains(previous.path.as_str())
            && !next_records
                .iter()
                .any(|record| record.path == previous.path)
        {
            next_records.push(previous.clone());
        }
    }

    next_records.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(next_records)
}

fn render_canonical_listing(target_url: &str, target_key: &str, entries: &[FileEntry]) -> String {
    let sorted_entries = sorted_entries(entries);
    let mut content = String::new();
    content.push_str(&format!("CRAWL LISTING FOR: {}\n", target_url));
    content.push_str(&format!("TARGET KEY: {}\n", target_key));
    content.push_str(&format!(
        "CRAWL INDEX COMPLETED AT: {}\n",
        chrono::Local::now().to_rfc2822()
    ));
    content.push_str(&format!("TOTAL ENTRIES: {}\n", sorted_entries.len()));
    content
        .push_str("========================================================================\n\n");

    for entry in sorted_entries {
        let type_str = if matches!(entry.entry_type, EntryType::Folder) {
            "[DIR]"
        } else {
            "[FILE]"
        };
        let size_str = entry
            .size_bytes
            .map(|size| format!("{} bytes", size))
            .unwrap_or_else(|| "Unknown size".to_string());
        content.push_str(&format!("{:<7} {} ({})\n", type_str, entry.path, size_str));
    }

    content
}

fn render_dir_listing(target_url: &str, target_key: &str, entries: &[FileEntry]) -> String {
    let sorted_entries = sorted_entries(entries);
    let mut content = String::new();
    content.push_str(&format!("DIR /S STYLE LISTING FOR: {}\n", target_url));
    content.push_str(&format!("TARGET KEY: {}\n", target_key));
    content.push_str(&format!(
        "GENERATED AT: {}\n",
        chrono::Local::now().to_rfc2822()
    ));
    content.push_str(&format!("TOTAL ENTRIES: {}\n", sorted_entries.len()));
    content
        .push_str("========================================================================\n\n");

    for entry in sorted_entries {
        let windows_path = entry.path.trim_start_matches('/').replace('/', "\\");
        match entry.entry_type {
            EntryType::Folder => {
                content.push_str(&format!("<DIR>          {}\n", windows_path));
            }
            EntryType::File => {
                let size = entry.size_bytes.unwrap_or(0);
                content.push_str(&format!("{:>14} {}\n", size, windows_path));
            }
        }
    }

    content
}

fn sorted_entries(entries: &[FileEntry]) -> Vec<FileEntry> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| match (&a.entry_type, &b.entry_type) {
        (EntryType::Folder, EntryType::File) => std::cmp::Ordering::Less,
        (EntryType::File, EntryType::Folder) => std::cmp::Ordering::Greater,
        _ => a.path.cmp(&b.path),
    });
    sorted
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileDownloadState {
    ExactMatch,
    NeedsDownload,
    PresentWithoutSize,
}

fn file_download_state(output_root: &Path, entry: &FileEntry) -> Result<FileDownloadState> {
    let target = path_utils::resolve_path_within_root(output_root, &entry.path, false)?
        .ok_or_else(|| anyhow::anyhow!("resolved download target is empty"))?;
    let metadata = match std::fs::metadata(&target) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(FileDownloadState::NeedsDownload),
    };

    match entry.size_bytes {
        Some(size_hint) if size_hint > 0 => {
            if metadata.len() == size_hint {
                Ok(FileDownloadState::ExactMatch)
            } else {
                Ok(FileDownloadState::NeedsDownload)
            }
        }
        _ => Ok(FileDownloadState::PresentWithoutSize),
    }
}

fn normalized_target_url(url: &str) -> String {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        let scheme = parsed.scheme().to_ascii_lowercase();
        let mut host = parsed
            .host_str()
            .map(|host| host.to_ascii_lowercase())
            .unwrap_or_default();
        if let Some(port) = parsed.port() {
            host = format!("{host}:{port}");
        }
        let mut path = parsed.path().to_string();
        if path.is_empty() {
            path.push('/');
        } else if path.len() > 1 {
            path = path.trim_end_matches('/').to_string();
            if path.is_empty() {
                path.push('/');
            }
        }
        let query = parsed
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
        return format!("{scheme}://{host}{path}{query}");
    }

    url.trim().to_ascii_lowercase()
}

fn logical_id_from_parsed_url(parsed: &reqwest::Url) -> Option<String> {
    if let Some((_, uuid)) = parsed.query_pairs().find(|(key, _)| key == "uuid") {
        let value = uuid.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }

    parsed
        .path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .map(path_utils::url_decode)
        .filter(|segment| !segment.is_empty())
}

fn extract_host_fallback(url: &str) -> String {
    url.trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("unknown-host")
        .to_ascii_lowercase()
}

fn sanitize_key_component(raw: &str, max_len: usize) -> String {
    let sanitized = path_utils::sanitize_path(raw)
        .replace('/', "_")
        .replace('\\', "_");
    let lowered = sanitized.to_ascii_lowercase();
    let trimmed = lowered.trim_matches('_');
    let mut component = if trimmed.is_empty() {
        "root".to_string()
    } else {
        trimmed.to_string()
    };
    if component.len() > max_len {
        component.truncate(max_len);
    }
    component
}

fn short_hash(input: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:08x}", hasher.finish())
}

fn should_replace_entry(existing: &FileEntry, candidate: &FileEntry) -> bool {
    (existing.size_bytes.unwrap_or(0) == 0 && candidate.size_bytes.unwrap_or(0) > 0)
        || (existing.raw_url.is_empty() && !candidate.raw_url.is_empty())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("crawli_target_state_test_{}_{}", name, unix_now()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(path).unwrap()
    }

    fn entry(path: &str, size: Option<u64>) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            size_bytes: size,
            entry_type: if path.ends_with('/') {
                EntryType::Folder
            } else {
                EntryType::File
            },
            raw_url: format!("http://fixture.onion{}", path),
        }
    }

    #[test]
    fn target_key_is_stable_for_same_url() {
        let a = derive_target_identity(
            "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=abc",
        );
        let b = derive_target_identity(
            "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=abc",
        );
        assert_eq!(a.target_key, b.target_key);
    }

    #[test]
    fn target_key_changes_for_different_url() {
        let a = derive_target_identity("http://examplea.onion/files/root");
        let b = derive_target_identity("http://exampleb.onion/files/root");
        assert_ne!(a.target_key, b.target_key);
    }

    #[test]
    fn merge_entries_promotes_better_metadata_without_losing_prior_paths() {
        let best = vec![entry("/dir/file.bin", None)];
        let current = vec![
            entry("/dir/file.bin", Some(512)),
            entry("/dir/new.bin", Some(1024)),
        ];
        let merged = merge_entries(&best, &current);

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged
                .iter()
                .find(|item| item.path == "/dir/file.bin")
                .and_then(|item| item.size_bytes),
            Some(512)
        );
    }

    #[test]
    fn download_resume_plan_prioritizes_failures_before_remaining_missing_items() {
        let temp = temp_root("resume_plan");
        let entries = vec![
            entry("/one.bin", Some(10)),
            entry("/two.bin", Some(20)),
            entry("/three.bin", Some(30)),
        ];
        let exact = path_utils::resolve_path_within_root(&temp, "/three.bin", false)
            .unwrap()
            .unwrap();
        std::fs::create_dir_all(exact.parent().unwrap()).unwrap();
        std::fs::write(&exact, vec![0u8; 30]).unwrap();

        let failures = vec![DownloadFailureRecord {
            path: "/two.bin".to_string(),
            source_url: "http://fixture.onion/two.bin".to_string(),
            size_hint: Some(20),
            last_error: "timeout".to_string(),
            stage: "failed".to_string(),
            attempt_count: 1,
            last_attempt_epoch: unix_now(),
        }];

        let build = build_download_resume_plan(
            "fixture_target",
            &entries,
            &failures,
            &temp,
            &temp.join("download_failures.json"),
        )
        .unwrap();

        assert_eq!(build.plan.failed_first_count, 1);
        assert_eq!(build.plan.missing_or_mismatch_count, 1);
        assert_eq!(build.plan.skipped_exact_matches_count, 1);
        assert_eq!(build.ordered_entries[0].path, "/two.bin");
        assert_eq!(build.ordered_entries[1].path, "/one.bin");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn reconcile_failure_manifest_drops_successful_entries() {
        let temp = temp_root("reconcile");
        let entries = vec![entry("/ok.bin", Some(8)), entry("/fail.bin", Some(12))];
        let ok_path = path_utils::resolve_path_within_root(&temp, "/ok.bin", false)
            .unwrap()
            .unwrap();
        let fail_path = path_utils::resolve_path_within_root(&temp, "/fail.bin", false)
            .unwrap()
            .unwrap();
        std::fs::write(&ok_path, vec![0u8; 8]).unwrap();
        std::fs::write(&fail_path, vec![0u8; 3]).unwrap();

        let previous = vec![
            DownloadFailureRecord {
                path: "/ok.bin".to_string(),
                source_url: "http://fixture.onion/ok.bin".to_string(),
                size_hint: Some(8),
                last_error: "old".to_string(),
                stage: "failed".to_string(),
                attempt_count: 1,
                last_attempt_epoch: 1,
            },
            DownloadFailureRecord {
                path: "/fail.bin".to_string(),
                source_url: "http://fixture.onion/fail.bin".to_string(),
                size_hint: Some(12),
                last_error: "old".to_string(),
                stage: "failed".to_string(),
                attempt_count: 1,
                last_attempt_epoch: 1,
            },
        ];

        let next =
            reconcile_failure_manifest(&previous, &entries, &entries, &temp, "failed").unwrap();

        assert_eq!(next.len(), 1);
        assert_eq!(next[0].path, "/fail.bin");
        let _ = std::fs::remove_dir_all(&temp);
    }
}
