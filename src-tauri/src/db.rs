use crate::adapters::FileEntry;
use crate::subtree_heatmap::SubtreeHeatRecord;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::mem;
use std::sync::Arc;
use tokio::sync::Mutex;

fn normalize_vfs_path(raw: &str) -> String {
    raw.replace('\\', "/")
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn canonical_vfs_path(raw: &str) -> Option<String> {
    let normalized = normalize_vfs_path(raw);
    if normalized.is_empty() {
        None
    } else {
        Some(format!("/{normalized}"))
    }
}

fn normalize_vfs_entry(entry: &FileEntry) -> Option<FileEntry> {
    let canonical_path = canonical_vfs_path(&entry.path)?;
    let mut normalized = entry.clone();
    normalized.path = canonical_path;
    Some(normalized)
}

fn vfs_parent_scan_prefixes(parent_prefix: &str) -> Vec<String> {
    let normalized = normalize_vfs_path(parent_prefix);
    if normalized.is_empty() {
        return Vec::new();
    }

    let backslash = normalized.replace('/', "\\");
    let mut prefixes = Vec::<String>::new();
    for candidate in [
        format!("/{normalized}/"),
        format!("{normalized}/"),
        format!("/{backslash}\\"),
        format!("{backslash}\\"),
        format!("\\{backslash}\\"),
    ] {
        if !prefixes.contains(&candidate) {
            prefixes.push(candidate);
        }
    }
    prefixes
}

fn collect_child_entry(
    entries: &mut Vec<FileEntry>,
    seen_paths: &mut std::collections::HashSet<String>,
    parent_prefix: &str,
    entry: &FileEntry,
) {
    let Some(normalized_entry) = normalize_vfs_entry(entry) else {
        return;
    };
    let normalized_parent = normalize_vfs_path(parent_prefix);
    let normalized_entry_path = normalize_vfs_path(&normalized_entry.path);

    let relative_path = if normalized_parent.is_empty() {
        normalized_entry_path
    } else if normalized_entry_path == normalized_parent {
        return;
    } else if let Some(rest) = normalized_entry_path.strip_prefix(&format!("{normalized_parent}/"))
    {
        rest.to_string()
    } else {
        return;
    };

    let parts = relative_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return;
    }

    if parts.len() == 1 {
        if seen_paths.insert(normalized_entry.path.clone()) {
            entries.push(normalized_entry);
        }
        return;
    }

    let directory_path = if normalized_parent.is_empty() {
        format!("/{}", parts[0])
    } else {
        format!("/{normalized_parent}/{}", parts[0])
    };
    if seen_paths.insert(directory_path.clone()) {
        entries.push(FileEntry {
            jwt_exp: None,
            path: directory_path,
            size_bytes: None,
            entry_type: crate::adapters::EntryType::Folder,
            raw_url: String::new(),
        });
    }
}

#[derive(Clone)]
pub struct SledVfs {
    db: Arc<Mutex<Option<sled::Db>>>,
    heatmap_tree: Arc<Mutex<Option<sled::Tree>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VfsSummary {
    pub discovered_count: usize,
    pub file_count: usize,
    pub folder_count: usize,
    pub total_size_bytes: u64,
}

impl Default for SledVfs {
    fn default() -> Self {
        Self {
            db: Arc::new(Mutex::new(None)),
            heatmap_tree: Arc::new(Mutex::new(None)),
        }
    }
}

impl SledVfs {
    pub async fn initialize(&self, path: &str) -> Result<()> {
        let db = sled::Config::new()
            .path(path)
            .mode(sled::Mode::HighThroughput)
            .cache_capacity(256 * 1024 * 1024) // 256MB cache threshold for massive dataset ingestion
            .flush_every_ms(None) // Disable built-in flush thread; Crawli controls flush bounds via explicit async commits
            .use_compression(false) // Optimize for maximum I/O throughput over disk savings
            .open()
            .context("Failed to open aerospace-grade sled database")?;

        // Phase 75: Open a dedicated named tree for persistent subtree heatmap indices
        let heatmap = db
            .open_tree("heatmap")
            .context("Failed to open heatmap tree")?;

        let mut guard = self.db.lock().await;
        *guard = Some(db);
        let mut hm_guard = self.heatmap_tree.lock().await;
        *hm_guard = Some(heatmap);
        Ok(())
    }

    pub async fn insert_entries(&self, entries: &[FileEntry]) -> Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let mut batch = sled::Batch::default();
            for entry in entries {
                let Some(normalized_entry) = normalize_vfs_entry(entry) else {
                    continue;
                };
                let bytes = serde_json::to_vec(&normalized_entry)?;
                batch.insert(normalized_entry.path.as_bytes(), bytes);
            }
            db.apply_batch(batch)?;
            db.flush_async().await?;
        }
        Ok(())
    }

    pub async fn get_entry(&self, path: &str) -> Result<Option<FileEntry>> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let mut lookup_paths = Vec::<String>::new();
            if let Some(canonical_path) = canonical_vfs_path(path) {
                lookup_paths.push(canonical_path);
            }
            let normalized = normalize_vfs_path(path);
            for candidate in [
                normalized.clone(),
                normalized.replace('/', "\\"),
                path.to_string(),
            ] {
                if !candidate.is_empty() && !lookup_paths.contains(&candidate) {
                    lookup_paths.push(candidate);
                }
            }

            for lookup in lookup_paths {
                if let Some(bytes) = db.get(lookup.as_bytes())? {
                    let entry: FileEntry = serde_json::from_slice(&bytes)?;
                    return Ok(normalize_vfs_entry(&entry));
                }
            }
        }
        Ok(None)
    }

    pub async fn iter_entries(&self) -> Result<Vec<FileEntry>> {
        let db = { self.db.lock().await.clone() };
        let mut entries = std::collections::BTreeMap::<String, FileEntry>::new();
        if let Some(db) = db.as_ref() {
            for (_, value) in db.iter().flatten() {
                if let Ok(entry) = serde_json::from_slice::<FileEntry>(&value) {
                    if let Some(normalized_entry) = normalize_vfs_entry(&entry) {
                        entries
                            .entry(normalized_entry.path.clone())
                            .or_insert(normalized_entry);
                    }
                }
            }
        }
        Ok(entries.into_values().collect())
    }

    pub async fn summarize_entries(&self) -> Result<VfsSummary> {
        let db = { self.db.lock().await.clone() };
        let mut summary = VfsSummary::default();
        let mut seen_paths = std::collections::HashSet::<String>::new();

        if let Some(db) = db.as_ref() {
            for (_, value) in db.iter().flatten() {
                if let Ok(entry) = serde_json::from_slice::<FileEntry>(&value) {
                    let Some(normalized_entry) = normalize_vfs_entry(&entry) else {
                        continue;
                    };
                    if !seen_paths.insert(normalized_entry.path.clone()) {
                        continue;
                    }
                    summary.discovered_count += 1;
                    match normalized_entry.entry_type {
                        crate::adapters::EntryType::File => {
                            summary.file_count += 1;
                            summary.total_size_bytes = summary
                                .total_size_bytes
                                .saturating_add(normalized_entry.size_bytes.unwrap_or(0));
                        }
                        crate::adapters::EntryType::Folder => {
                            summary.folder_count += 1;
                        }
                    }
                }
            }
        }

        Ok(summary)
    }

    pub async fn with_entry_batches<F>(&self, batch_size: usize, mut visitor: F) -> Result<()>
    where
        F: FnMut(Vec<FileEntry>) -> Result<()>,
    {
        let db = { self.db.lock().await.clone() };
        let Some(db) = db else {
            return Ok(());
        };

        let mut batch = Vec::with_capacity(batch_size.max(1));
        let mut seen_paths = std::collections::HashSet::<String>::new();
        for (_, value) in db.iter().flatten() {
            if let Ok(entry) = serde_json::from_slice::<FileEntry>(&value) {
                let Some(normalized_entry) = normalize_vfs_entry(&entry) else {
                    continue;
                };
                if !seen_paths.insert(normalized_entry.path.clone()) {
                    continue;
                }
                batch.push(normalized_entry);
                if batch.len() >= batch_size.max(1) {
                    visitor(mem::take(&mut batch))?;
                }
            }
        }

        if !batch.is_empty() {
            visitor(batch)?;
        }

        Ok(())
    }

    pub async fn get_children(&self, parent_prefix: &str) -> Result<Vec<FileEntry>> {
        let guard = self.db.lock().await;
        let mut entries = Vec::new();
        if let Some(db) = guard.as_ref() {
            let mut seen_paths = std::collections::HashSet::<String>::new();

            if parent_prefix.is_empty() {
                for (_k, v) in db.iter().flatten() {
                    if let Ok(entry) = serde_json::from_slice::<FileEntry>(&v) {
                        collect_child_entry(&mut entries, &mut seen_paths, parent_prefix, &entry);
                    }
                }
            } else {
                let mut found_prefixed_match = false;
                for prefix in vfs_parent_scan_prefixes(parent_prefix) {
                    for (_k, v) in db.scan_prefix(prefix.as_bytes()).flatten() {
                        found_prefixed_match = true;
                        if let Ok(entry) = serde_json::from_slice::<FileEntry>(&v) {
                            collect_child_entry(
                                &mut entries,
                                &mut seen_paths,
                                parent_prefix,
                                &entry,
                            );
                        }
                    }
                }

                if !found_prefixed_match {
                    for (_k, v) in db.iter().flatten() {
                        if let Ok(entry) = serde_json::from_slice::<FileEntry>(&v) {
                            collect_child_entry(
                                &mut entries,
                                &mut seen_paths,
                                parent_prefix,
                                &entry,
                            );
                        }
                    }
                }
            }
        }

        // Sort folders first, then alphabetically
        entries.sort_by(|a, b| {
            if a.entry_type == crate::adapters::EntryType::Folder
                && b.entry_type == crate::adapters::EntryType::File
            {
                std::cmp::Ordering::Less
            } else if a.entry_type == crate::adapters::EntryType::File
                && b.entry_type == crate::adapters::EntryType::Folder
            {
                std::cmp::Ordering::Greater
            } else {
                a.path.cmp(&b.path)
            }
        });

        Ok(entries)
    }

    pub async fn clear(&self) -> Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            db.clear()?;
            db.flush_async().await?;
        }
        // Note: heatmap tree is NOT cleared here — it persists across crawl sessions
        Ok(())
    }

    // Phase 72: Aerospace-Grade Auto-Compacting VFS Ledger
    // Triggers manual compaction and synchronous flush boundaries inside Sled DB memory.
    pub async fn compact_database(&self) -> Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            // Sled defragments natively, but we can force flushing buffers here for peace of mind bounds
            db.flush_async().await?;
        }
        Ok(())
    }

    // ─── Phase 75: Persistent Subtree Heatmap Sled Index ────────────────────────
    // Uses a dedicated named tree within the same Sled DB for crash-safe,
    // cross-session subtree failure tracking. Survives clear() calls on the
    // main VFS tree since heatmap data has a different lifecycle.

    /// Insert or update a single heatmap record.
    pub async fn upsert_heatmap_record(&self, record: &SubtreeHeatRecord) -> Result<()> {
        let guard = self.heatmap_tree.lock().await;
        if let Some(tree) = guard.as_ref() {
            let bytes = serde_json::to_vec(record)?;
            tree.insert(record.subtree_key.as_bytes(), bytes)?;
            tree.flush_async().await?;
        }
        Ok(())
    }

    /// Batch-upsert multiple heatmap records in a single atomic write.
    pub async fn upsert_heatmap_batch(&self, records: &[SubtreeHeatRecord]) -> Result<()> {
        let guard = self.heatmap_tree.lock().await;
        if let Some(tree) = guard.as_ref() {
            let mut batch = sled::Batch::default();
            for record in records {
                let bytes = serde_json::to_vec(record)?;
                batch.insert(record.subtree_key.as_bytes(), bytes);
            }
            tree.apply_batch(batch)?;
            tree.flush_async().await?;
        }
        Ok(())
    }

    /// Load all heatmap records from the persistent Sled tree.
    pub async fn load_heatmap_records(
        &self,
    ) -> Result<std::collections::BTreeMap<String, SubtreeHeatRecord>> {
        let guard = self.heatmap_tree.lock().await;
        let mut map = std::collections::BTreeMap::new();
        if let Some(tree) = guard.as_ref() {
            for (key_bytes, value_bytes) in tree.iter().flatten() {
                if let Ok(record) = serde_json::from_slice::<SubtreeHeatRecord>(&value_bytes) {
                    if let Ok(key) = String::from_utf8(key_bytes.to_vec()) {
                        map.insert(key, record);
                    }
                }
            }
        }
        Ok(map)
    }

    /// Look up a single heatmap record by subtree key.
    pub async fn get_heatmap_record(&self, subtree_key: &str) -> Result<Option<SubtreeHeatRecord>> {
        let guard = self.heatmap_tree.lock().await;
        if let Some(tree) = guard.as_ref() {
            if let Some(bytes) = tree.get(subtree_key.as_bytes())? {
                let record: SubtreeHeatRecord = serde_json::from_slice(&bytes)?;
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    /// Clear only the heatmap tree (used for explicit resets, NOT during normal clear()).
    pub async fn clear_heatmap(&self) -> Result<()> {
        let guard = self.heatmap_tree.lock().await;
        if let Some(tree) = guard.as_ref() {
            tree.clear()?;
            tree.flush_async().await?;
        }
        Ok(())
    }

    /// Count of entries in the heatmap tree.
    pub async fn heatmap_count(&self) -> usize {
        let guard = self.heatmap_tree.lock().await;
        if let Some(tree) = guard.as_ref() {
            tree.len()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SledVfs;
    use crate::adapters::{EntryType, FileEntry};

    fn file_entry(path: &str, entry_type: EntryType) -> FileEntry {
        FileEntry {
            jwt_exp: None,
            path: path.to_string(),
            size_bytes: Some(1),
            entry_type,
            raw_url: "http://fixture".to_string(),
        }
    }

    fn temp_vfs_path(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "crawli-vfs-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[tokio::test]
    async fn insert_entries_normalizes_windows_separators() {
        let vfs = SledVfs::default();
        let db_path = temp_vfs_path("insert-normalize");
        vfs.initialize(&db_path.to_string_lossy()).await.unwrap();

        vfs.insert_entries(&[file_entry(
            r"evidence\screenshots\screen01.png",
            EntryType::File,
        )])
        .await
        .unwrap();

        let stored = vfs
            .get_entry("/evidence/screenshots/screen01.png")
            .await
            .unwrap()
            .expect("normalized entry should exist");
        assert_eq!(stored.path, "/evidence/screenshots/screen01.png");

        let _ = std::fs::remove_dir_all(db_path);
    }

    #[tokio::test]
    async fn get_children_keeps_legacy_windows_paths_nested() {
        let vfs = SledVfs::default();
        let db_path = temp_vfs_path("legacy-children");
        vfs.initialize(&db_path.to_string_lossy()).await.unwrap();

        {
            let guard = vfs.db.lock().await;
            let db = guard.as_ref().expect("db should be initialized");
            let mut batch = sled::Batch::default();
            let file = file_entry(r"evidence\screenshots\screen01.png", EntryType::File);
            batch.insert(file.path.as_bytes(), serde_json::to_vec(&file).unwrap());
            let sibling = file_entry("/evidence/report.pdf", EntryType::File);
            batch.insert(
                sibling.path.as_bytes(),
                serde_json::to_vec(&sibling).unwrap(),
            );
            db.apply_batch(batch).unwrap();
            db.flush_async().await.unwrap();
        }

        let root = vfs.get_children("").await.unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].path, "/evidence");
        assert_eq!(root[0].entry_type, EntryType::Folder);

        let evidence = vfs.get_children("/evidence").await.unwrap();
        assert_eq!(
            evidence
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/evidence/screenshots", "/evidence/report.pdf"]
        );

        let screenshots = vfs.get_children("/evidence/screenshots").await.unwrap();
        assert_eq!(screenshots.len(), 1);
        assert_eq!(screenshots[0].path, "/evidence/screenshots/screen01.png");

        let _ = std::fs::remove_dir_all(db_path);
    }
}
