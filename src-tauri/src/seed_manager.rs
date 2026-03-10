use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SeedManager {
    primary_seed: tokio::sync::RwLock<String>,
    fallback_seeds: tokio::sync::RwLock<Vec<String>>,
    consecutive_failures: AtomicUsize,
}

impl SeedManager {
    pub fn new(primary: String, fallbacks: Vec<String>) -> Self {
        Self {
            primary_seed: tokio::sync::RwLock::new(primary),
            fallback_seeds: tokio::sync::RwLock::new(fallbacks),
            consecutive_failures: AtomicUsize::new(0),
        }
    }

    pub async fn get_active_seed(&self) -> String {
        self.primary_seed.read().await.clone()
    }

    pub fn get_active_seed_sync(&self) -> String {
        self.primary_seed
            .try_read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub async fn report_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    pub async fn report_failure(&self, threshold: usize) -> bool {
        let fails = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if fails >= threshold {
            self.consecutive_failures.store(0, Ordering::Relaxed);
            self.rotate_seed().await
        } else {
            false
        }
    }

    async fn rotate_seed(&self) -> bool {
        let mut primary = self.primary_seed.write().await;
        let mut fallbacks = self.fallback_seeds.write().await;

        if !fallbacks.is_empty() {
            let next = fallbacks.remove(0);
            fallbacks.push(primary.clone()); // put old primary at back
            *primary = next;
            true
        } else {
            false
        }
    }

    pub async fn add_fallbacks(&self, mut new_fallbacks: Vec<String>) {
        let primary = self.primary_seed.read().await.clone();
        new_fallbacks.retain(|url| url != &primary);
        let mut fallbacks = self.fallback_seeds.write().await;
        *fallbacks = new_fallbacks;
    }

    pub async fn remap_url(&self, current_url: &str, original_seed: &str) -> String {
        let active = self.get_active_seed().await;
        if current_url.starts_with(&active) {
            current_url.to_string()
        } else if let Some(stripped) = current_url.strip_prefix(original_seed) {
            format!("{}{}", active, stripped)
        } else {
            current_url.to_string()
        }
    }
}
