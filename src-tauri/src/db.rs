use crate::adapters::FileEntry;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::mem;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SledVfs {
    db: Arc<Mutex<Option<sled::Db>>>,
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
        }
    }
}

impl SledVfs {
    pub async fn initialize(&self, path: &str) -> Result<()> {
        let db = sled::open(path).context("Failed to open sled database")?;
        let mut guard = self.db.lock().await;
        *guard = Some(db);
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
        Ok(())
    }
}
