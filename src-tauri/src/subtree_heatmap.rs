use crate::path_utils;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::RwLock;

/// Phase 76: Global heatmap state for cross-thread `is_subtree_penalized()` access.
struct GlobalHeatmapState {
    active_seed_url: String,
    heatmap: SubtreeHeatmap,
}

/// Phase 76: The global static heatmap. Populated via `install_global_heatmap()` at crawl start.
static GLOBAL_HEATMAP: std::sync::LazyLock<RwLock<Option<GlobalHeatmapState>>> =
    std::sync::LazyLock::new(|| RwLock::new(None));

const MAX_ENTRIES: usize = 512;
const STALE_WINDOW_SECS: u64 = 14 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtreeHeatRecord {
    pub subtree_key: String,
    pub failure_score: u32,
    pub timeout_count: u32,
    pub circuit_count: u32,
    pub throttle_count: u32,
    pub http_count: u32,
    pub success_count: u32,
    pub consecutive_failures: u32,
    pub last_failure_epoch: u64,
    pub last_success_epoch: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistentSubtreeHeatmap {
    pub target_key: String,
    pub updated_at_epoch: u64,
    pub entries: BTreeMap<String, SubtreeHeatRecord>,
}

#[derive(Clone, Debug, Default)]
pub struct SubtreeHeatmap {
    pub target_key: String,
    pub entries: BTreeMap<String, SubtreeHeatRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeatFailureKind {
    Timeout,
    Circuit,
    Throttle,
    Http,
}

impl SubtreeHeatmap {
    pub fn load(path: &Path, target_key: &str) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                target_key: target_key.to_string(),
                entries: BTreeMap::new(),
            });
        }

        let bytes =
            std::fs::read(path).with_context(|| format!("read heatmap {}", path.display()))?;
        let persisted: PersistentSubtreeHeatmap = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse heatmap {}", path.display()))?;
        Ok(Self {
            target_key: target_key.to_string(),
            entries: persisted.entries,
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut entries = self.entries.clone();
        prune_entries(&mut entries);
        let payload = PersistentSubtreeHeatmap {
            target_key: self.target_key.clone(),
            updated_at_epoch: unix_now(),
            entries,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&payload)?)
            .with_context(|| format!("write heatmap {}", path.display()))?;
        Ok(())
    }

    pub fn subtree_key(active_seed_url: &str, target_url: &str) -> Option<String> {
        if !target_url.starts_with(active_seed_url) {
            return None;
        }

        let relative = &target_url[active_seed_url.len()..];
        let decoded = path_utils::url_decode(relative);
        let segments: Vec<String> = decoded
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .take(2)
            .map(path_utils::sanitize_path)
            .filter(|segment| !segment.is_empty())
            .collect();

        if segments.is_empty() {
            None
        } else {
            Some(segments.join("/"))
        }
    }

    pub fn should_route_to_degraded(&self, subtree_key: &str) -> bool {
        let Some(record) = self.entries.get(subtree_key) else {
            return false;
        };
        if is_stale(record.last_failure_epoch, record.last_success_epoch) {
            return false;
        }
        record.failure_score >= 4 || record.consecutive_failures >= 2
    }

    /// Phase 76: Wire is_subtree_penalized() to the live global heatmap.
    /// The global cache is populated via install_global_heatmap() at crawl start
    /// and cleared via uninstall_global_heatmap() at crawl end.
    pub fn is_subtree_penalized(url: &str) -> bool {
        let guard = match GLOBAL_HEATMAP.read() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let hm = match guard.as_ref() {
            Some(h) => h,
            None => return false,
        };

        let Some(key) = Self::subtree_key(&hm.active_seed_url, url) else {
            return false;
        };
        hm.heatmap.should_route_to_degraded(&key)
    }

    /// Phase 76: Install the live heatmap into the global cache so
    /// `is_subtree_penalized()` returns correct results from any thread.
    pub fn install_global_heatmap(active_seed_url: String, heatmap: SubtreeHeatmap) {
        if let Ok(mut guard) = GLOBAL_HEATMAP.write() {
            *guard = Some(GlobalHeatmapState {
                active_seed_url,
                heatmap,
            });
        }
    }

    /// Phase 76: Clear the global heatmap at crawl end.
    pub fn uninstall_global_heatmap() {
        if let Ok(mut guard) = GLOBAL_HEATMAP.write() {
            *guard = None;
        }
    }

    /// Phase 76: Update the global heatmap with fresh data (called periodically during crawl).
    pub fn refresh_global_heatmap(heatmap: &SubtreeHeatmap) {
        if let Ok(mut guard) = GLOBAL_HEATMAP.write() {
            if let Some(ref mut state) = *guard {
                state.heatmap = heatmap.clone();
            }
        }
    }

    pub fn record_failure(&mut self, subtree_key: &str, kind: HeatFailureKind) {
        let now = unix_now();
        let record = self
            .entries
            .entry(subtree_key.to_string())
            .or_insert_with(|| SubtreeHeatRecord {
                subtree_key: subtree_key.to_string(),
                ..Default::default()
            });

        record.consecutive_failures = record.consecutive_failures.saturating_add(1);
        record.last_failure_epoch = now;
        match kind {
            HeatFailureKind::Timeout => {
                record.timeout_count = record.timeout_count.saturating_add(1);
                record.failure_score = record.failure_score.saturating_add(3);
            }
            HeatFailureKind::Circuit => {
                record.circuit_count = record.circuit_count.saturating_add(1);
                record.failure_score = record.failure_score.saturating_add(2);
            }
            HeatFailureKind::Throttle => {
                record.throttle_count = record.throttle_count.saturating_add(1);
                record.failure_score = record.failure_score.saturating_add(2);
            }
            HeatFailureKind::Http => {
                record.http_count = record.http_count.saturating_add(1);
                record.failure_score = record.failure_score.saturating_add(1);
            }
        }
    }

    pub fn record_success(&mut self, subtree_key: &str) {
        let now = unix_now();
        let record = self
            .entries
            .entry(subtree_key.to_string())
            .or_insert_with(|| SubtreeHeatRecord {
                subtree_key: subtree_key.to_string(),
                ..Default::default()
            });
        record.success_count = record.success_count.saturating_add(1);
        record.consecutive_failures = 0;
        record.last_success_epoch = now;
        record.failure_score = record.failure_score.saturating_sub(1);
    }
}

fn prune_entries(entries: &mut BTreeMap<String, SubtreeHeatRecord>) {
    entries.retain(|_, record| !is_stale(record.last_failure_epoch, record.last_success_epoch));
    if entries.len() <= MAX_ENTRIES {
        return;
    }

    let mut scored = entries
        .values()
        .map(|record| {
            let weight = record.failure_score as u64
                + (record.consecutive_failures as u64 * 2)
                + record
                    .last_failure_epoch
                    .saturating_sub(record.last_success_epoch);
            (record.subtree_key.clone(), weight)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.truncate(MAX_ENTRIES);
    let keep = scored
        .into_iter()
        .map(|(key, _)| key)
        .collect::<std::collections::BTreeSet<_>>();
    entries.retain(|key, _| keep.contains(key));
}

fn is_stale(last_failure_epoch: u64, last_success_epoch: u64) -> bool {
    let last_seen = last_failure_epoch.max(last_success_epoch);
    last_seen > 0 && unix_now().saturating_sub(last_seen) > STALE_WINDOW_SECS
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{HeatFailureKind, SubtreeHeatmap};

    #[test]
    fn subtree_key_collapses_to_two_segments() {
        let key = SubtreeHeatmap::subtree_key(
            "http://host.onion/root/",
            "http://host.onion/root/Accounting/Bank%20Recs/2024/report.pdf",
        )
        .unwrap();
        assert_eq!(key, "Accounting/Bank Recs");
    }

    #[test]
    fn repeated_timeouts_mark_subtree_as_degraded() {
        let mut heatmap = SubtreeHeatmap::default();
        heatmap.record_failure("Accounting/Bank Recs", HeatFailureKind::Timeout);
        heatmap.record_failure("Accounting/Bank Recs", HeatFailureKind::Timeout);
        assert!(heatmap.should_route_to_degraded("Accounting/Bank Recs"));
    }

    #[test]
    fn success_decays_failure_score() {
        let mut heatmap = SubtreeHeatmap::default();
        heatmap.record_failure("T/Test Logic", HeatFailureKind::Circuit);
        heatmap.record_success("T/Test Logic");
        assert!(!heatmap.should_route_to_degraded("T/Test Logic"));
    }

    #[test]
    fn global_heatmap_lifecycle_install_refresh_uninstall() {
        // Clean slate
        SubtreeHeatmap::uninstall_global_heatmap();
        assert!(!SubtreeHeatmap::is_subtree_penalized(
            "http://x.onion/root/A/B/file.txt"
        ));

        // Install with degraded subtree
        let mut hm = SubtreeHeatmap {
            target_key: "test".to_string(),
            entries: Default::default(),
        };
        hm.record_failure("A/B", HeatFailureKind::Timeout);
        hm.record_failure("A/B", HeatFailureKind::Timeout);
        SubtreeHeatmap::install_global_heatmap("http://x.onion/root/".to_string(), hm.clone());
        assert!(SubtreeHeatmap::is_subtree_penalized(
            "http://x.onion/root/A/B/file.txt"
        ));

        // Refresh: add success to heal the subtree
        hm.record_success("A/B");
        hm.record_success("A/B");
        hm.record_success("A/B");
        hm.record_success("A/B");
        SubtreeHeatmap::refresh_global_heatmap(&hm);
        assert!(!SubtreeHeatmap::is_subtree_penalized(
            "http://x.onion/root/A/B/file.txt"
        ));

        // Uninstall: should return false for everything
        SubtreeHeatmap::uninstall_global_heatmap();
        assert!(!SubtreeHeatmap::is_subtree_penalized(
            "http://x.onion/root/A/B/file.txt"
        ));
    }
}
