use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::Path;

pub struct SpilloverQueue<T> {
    db: Db,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Serialize + for<'a> Deserialize<'a>> SpilloverQueue<T> {
    pub fn new() -> Self {
        let db = sled::Config::new()
            .temporary(true)
            .flush_every_ms(None)
            .open()
            .unwrap();
        Self {
            db,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn new_persistent(path: impl AsRef<Path>) -> Self {
        let db = sled::Config::new()
            .path(path)
            .flush_every_ms(None)
            .open()
            .unwrap();
        Self {
            db,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn push(&self, item: T) {
        if let Ok(bytes) = serde_json::to_vec(&item) {
            let idx = self.db.generate_id().unwrap_or(0);
            let _ = self.db.insert(idx.to_be_bytes(), bytes);
        }
    }

    pub fn push_batch(&self, items: Vec<T>) {
        if items.is_empty() { return; }
        let mut batch = sled::Batch::default();
        for item in items {
            if let Ok(bytes) = serde_json::to_vec(&item) {
                let idx = self.db.generate_id().unwrap_or(0);
                batch.insert(&idx.to_be_bytes(), bytes);
            }
        }
        let _ = self.db.apply_batch(batch);
    }

    pub fn pop(&self) -> Option<T> {
        loop {
            match self.db.pop_min() {
                Ok(Some((_k, bytes))) => {
                    if let Ok(item) = serde_json::from_slice(&bytes) {
                        return Some(item);
                    }
                    // Deserialization failed, try next
                }
                _ => return None,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    pub fn len(&self) -> usize {
        self.db.len()
    }
}

pub struct SpilloverList<T> {
    db: Db,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Serialize + for<'a> Deserialize<'a>> SpilloverList<T> {
    pub fn new() -> Self {
        let db = sled::Config::new()
            .temporary(true)
            .flush_every_ms(None)
            .open()
            .unwrap();
        Self {
            db,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn new_persistent(path: impl AsRef<Path>) -> Self {
        let db = sled::Config::new()
            .path(path)
            .flush_every_ms(None)
            .open()
            .unwrap();
        Self {
            db,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn push(&self, item: T) {
        if let Ok(bytes) = serde_json::to_vec(&item) {
            let idx = self.db.generate_id().unwrap_or(0);
            let _ = self.db.insert(idx.to_be_bytes(), bytes);
        }
    }

    pub fn push_batch(&self, items: Vec<T>) {
        if items.is_empty() { return; }
        let mut batch = sled::Batch::default();
        for item in items {
            if let Ok(bytes) = serde_json::to_vec(&item) {
                let idx = self.db.generate_id().unwrap_or(0);
                batch.insert(&idx.to_be_bytes(), bytes);
            }
        }
        let _ = self.db.apply_batch(batch);
    }

    pub fn drain_all(&self) -> Vec<T> {
        let mut results = Vec::new();
        while let Ok(Some((_k, v))) = self.db.pop_min() {
            if let Ok(decoded) = serde_json::from_slice(&v) {
                results.push(decoded);
            }
        }
        results
    }

    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    pub fn len(&self) -> usize {
        self.db.len()
    }
}
