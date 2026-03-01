use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use reqwest::header::HeaderMap;
use tauri::AppHandle;

/// The one-time snapshot of a website's initial page load.
/// Passed to every adapter to check if it matches their known architecture.
#[derive(Debug, Clone)]
pub struct SiteFingerprint {
    pub url: String,
    pub status: u16,
    pub headers: HeaderMap,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntryType {
    File,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,          // Relative path starting with /
    pub size_bytes: Option<u64>,
    pub entry_type: EntryType,
    pub raw_url: String,       // The actual URL to hit to download this, or crawl deeper
}

#[async_trait]
pub trait CrawlerAdapter: Send + Sync {
    /// Examine the site fingerprint and determine if this adapter can handle its DOM/Headers.
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool;
    
    /// Extract items from the given URL using the Frontier's Tor engine.
    /// Periodically emits progress back to the Tauri App to keep the UI responsive.
    async fn crawl(
        &self, 
        current_url: &str, 
        frontier: std::sync::Arc<crate::frontier::CrawlerFrontier>, 
        app: AppHandle
    ) -> anyhow::Result<Vec<FileEntry>>;
    
    /// Adapter name, for logging
    fn name(&self) -> &'static str;

    /// Database of known root `.onion` URLs that automatically bypass structural checks and resolve to this adapter instantly.
    fn known_domains(&self) -> Vec<&'static str> { vec![] }
}

pub mod worldleaks;
pub mod dragonforce;
pub mod lockbit;
pub mod nu;
pub mod autoindex;
pub mod inc_ransom;
pub mod pear;
pub mod play;

// Registry to hold all available adapters
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn CrawlerAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        let mut registry = AdapterRegistry {
            adapters: Vec::new(),
        };
        
        // Register all adapters — specific ones first, generic fallback last
        registry.adapters.push(Box::new(worldleaks::WorldLeaksAdapter::default()));
        registry.adapters.push(Box::new(dragonforce::DragonForceAdapter::default()));
        registry.adapters.push(Box::new(lockbit::LockBitAdapter::default()));
        registry.adapters.push(Box::new(nu::NuServerAdapter::default()));
        registry.adapters.push(Box::new(inc_ransom::IncRansomAdapter::default()));
        registry.adapters.push(Box::new(pear::PearAdapter::default()));
        registry.adapters.push(Box::new(play::PlayAdapter::default()));
        registry.adapters.push(Box::new(autoindex::AutoindexAdapter::default())); // Generic fallback — always last
        
        registry
    }

    pub async fn determine_adapter(&self, fingerprint: &SiteFingerprint) -> Option<&dyn CrawlerAdapter> {
        use futures::StreamExt;
        
        // 1. FAST PATH: Check O(1) known domain database mapped to the specific adapter
        for adapter in &self.adapters {
            for domain in adapter.known_domains() {
                if fingerprint.url.contains(domain) {
                    return Some(adapter.as_ref());
                }
            }
        }

        // 2. PARALLEL FALLBACK: If unknown URL, test structural fingerprints across all adapters concurrently.
        // The first one to validate wins, instantly dropping the other concurrent checks.
        let mut concurrent_checks = futures::stream::FuturesUnordered::new();
        for adapter in &self.adapters {
            concurrent_checks.push(async move {
                if adapter.can_handle(fingerprint).await {
                    Some(adapter.as_ref())
                } else {
                    None
                }
            });
        }

        while let Some(res) = concurrent_checks.next().await {
            if res.is_some() {
                return res;
            }
        }

        None
    }
}
