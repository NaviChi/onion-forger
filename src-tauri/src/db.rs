use crate::adapters::FileEntry;
use crate::subtree_heatmap::SubtreeHeatRecord;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::mem;
use std::sync::Arc;
use tokio::sync::Mutex;

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
                let bytes = serde_json::to_vec(entry)?;
                batch.insert(entry.path.as_bytes(), bytes);
            }
            db.apply_batch(batch)?;
            db.flush_async().await?;
        }
        Ok(())
    }

    pub async fn get_entry(&self, path: &str) -> Result<Option<FileEntry>> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            if let Some(bytes) = db.get(path.as_bytes())? {
                let entry: FileEntry = serde_json::from_slice(&bytes)?;
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    pub async fn iter_entries(&self) -> Result<Vec<FileEntry>> {
        let db = { self.db.lock().await.clone() };
        let mut entries = Vec::new();
        if let Some(db) = db.as_ref() {
            for (_, value) in db.iter().flatten() {
                if let Ok(entry) = serde_json::from_slice::<FileEntry>(&value) {
                    entries.push(entry);
                }
            }
        }
        Ok(entries)
    }

    pub async fn summarize_entries(&self) -> Result<VfsSummary> {
        let db = { self.db.lock().await.clone() };
        let mut summary = VfsSummary::default();

        if let Some(db) = db.as_ref() {
            for (_, value) in db.iter().flatten() {
                if let Ok(entry) = serde_json::from_slice::<FileEntry>(&value) {
                    summary.discovered_count += 1;
                    match entry.entry_type {
                        crate::adapters::EntryType::File => {
                            summary.file_count += 1;
                            summary.total_size_bytes = summary
                                .total_size_bytes
                                .saturating_add(entry.size_bytes.unwrap_or(0));
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
        for (_, value) in db.iter().flatten() {
            if let Ok(entry) = serde_json::from_slice::<FileEntry>(&value) {
                batch.push(entry);
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
            // Ensure prefix ends with a slash for accurate scoping, except root which is empty
            let mut prefix = parent_prefix.to_string();
            if !prefix.is_empty() && !prefix.ends_with('/') {
                prefix.push('/');
            }

            let mut seen_dirs = std::collections::HashSet::new();

            for (_k, v) in db.scan_prefix(prefix.as_bytes()).flatten() {
                if let Ok(entry) = serde_json::from_slice::<FileEntry>(&v) {
                    // Extract relative part after prefix
                    let relative_path = if prefix.is_empty() {
                        entry.path.clone()
                    } else if entry.path.starts_with(&prefix) {
                        entry.path[prefix.len()..].to_string()
                    } else {
                        continue;
                    };

                    let relative_path = relative_path.trim_start_matches('/');
                    let parts: Vec<&str> = relative_path.split('/').collect();

                    if parts.is_empty() || parts[0].is_empty() {
                        continue;
                    }

                    if parts.len() == 1 {
                        // Direct child file or empty dir
                        if entry.entry_type == crate::adapters::EntryType::Folder {
                            let dir_name = parts[0].to_string();
                            if !seen_dirs.contains(&dir_name) {
                                seen_dirs.insert(dir_name);
                                entries.push(entry);
                            }
                        } else {
                            entries.push(entry);
                        }
                    } else {
                        // It's a subdirectory, construct a virtual Folder entry if not seen
                        let dir_name = parts[0].to_string();
                        if !seen_dirs.contains(&dir_name) {
                            seen_dirs.insert(dir_name.clone());
                            let virtual_dir_path = format!("{}{}", prefix, dir_name);
                            entries.push(FileEntry {
                                jwt_exp: None,
                                path: virtual_dir_path,
                                size_bytes: None,
                                entry_type: crate::adapters::EntryType::Folder,
                                raw_url: "".to_string(), // Virtual folders don't have direct raw URLs
                            });
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
