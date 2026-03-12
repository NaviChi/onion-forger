use async_trait::async_trait;
use http::header::HeaderMap;
use serde::{Deserialize, Serialize};
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
    pub jwt_exp: Option<u64>,
    pub path: String, // Relative path starting with /
    pub size_bytes: Option<u64>,
    pub entry_type: EntryType,
    pub raw_url: String, // The actual URL to hit to download this, or crawl deeper
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterSupportInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub support_level: &'static str,
    pub matching_strategy: &'static str,
    pub sample_urls: Vec<&'static str>,
    pub tested_for: Vec<&'static str>,
    pub notes: &'static str,
}

#[async_trait]
pub trait CrawlerAdapter: Send + Sync {
    /// Examine the site fingerprint and determine if this adapter can handle its DOM/Headers.
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool;

    /// Extrac items from the given URL using the Frontier's Tor engine.
    /// Periodically emits progress back to the Tauri App to keep the UI responsive.
    async fn crawl(
        &self,
        current_url: &str,
        frontier: std::sync::Arc<crate::frontier::CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>>;

    /// Optional hook to fetch a fresh JWT token for an expired FileEntry.
    /// By default, returns Ok(None) meaning JWT refresh is unsupported or unnecessary.
    async fn refresh_jwt(
        &self,
        _entry: &FileEntry,
        _client: &crate::arti_client::ArtiClient,
    ) -> anyhow::Result<Option<FileEntry>> {
        Ok(None)
    }

    /// Adapter name, for logging
    fn name(&self) -> &'static str;

    /// Database of known root `.onion` URLs that automatically bypass structural checks and resolve to this adapter instantly.
    fn known_domains(&self) -> Vec<&'static str> {
        vec![]
    }

    /// Tier 2 Regex Bouncer Marker. The RegexSet engine will parse the raw HTML exactly once to pre-filter
    /// adapters before invoking concurrent `can_handle()` AST routines.
    fn regex_marker(&self) -> Option<&'static str> {
        None
    }
}

pub mod abyss;
pub mod adapter_pipeline_trait;
pub mod alphalocker;
pub mod autoindex;
pub mod dragonforce;
pub mod explorer;
pub mod genesis;
pub mod inc_ransom;
pub mod lockbit;
pub mod nu;
pub mod pear;
pub mod play;
pub mod plugin_host;
pub mod qilin;
pub mod qilin_ddos_guard;
pub mod qilin_nodes;
pub mod tengu;
pub mod universal_explorer;
pub mod worldleaks;

pub fn support_catalog() -> Vec<AdapterSupportInfo> {
    vec![
        AdapterSupportInfo {
            id: "qilin",
            name: "Qilin Nginx Autoindex",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain + QData marker signature matching",
            sample_urls: vec![
                "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/",
            ],
            tested_for: vec![
                "Adapter fingerprint match (engine_test)",
                "Autoindex traversal delegation (qilin adapter)",
            ],
            notes: "Uses hardened autoindex crawler for full recursive traversal and size mapping of the themed QData UI.",
        },
        AdapterSupportInfo {
            id: "worldleaks",
            name: "WorldLeaks SPA",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain and SPA fingerprint matching",
            sample_urls: vec!["http://worldleaks.onion"],
            tested_for: vec!["Adapter fingerprint match (engine_test)"],
            notes: "Production adapter with crawl traversal and progress streaming.",
        },
        AdapterSupportInfo {
            id: "dragonforce",
            name: "DragonForce Iframe SPA",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain and body marker matching",
            sample_urls: vec![
                "http://dragonforce.onion",
                "fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion",
            ],
            tested_for: vec![
                "Adapter fingerprint match (engine_test)",
                "Parser extraction flow (dragon_cli_test)",
            ],
            notes: "Production adapter for iframe and tokenized listing layouts.",
        },
        AdapterSupportInfo {
            id: "lockbit",
            name: "LockBit Embedded Nginx",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain + Nginx marker and body signature matching",
            sample_urls: vec![
                "http://lockbit.onion",
                "http://lockbit6vhrjaqzsdj6pqalyideigxv4xycfeyunpx35znogiwmojnid.onion/secret/212f70e703d758fbccbda3013a21f5de-f033da37-5fa7-31df-b10c-cc04b8538e85/jobberswarehouse.com/",
            ],
            tested_for: vec![
                "Adapter fingerprint match (engine_test)",
                "Direct artifact URL routing (engine_test)",
                "Autoindex traversal delegation (lockbit adapter)",
            ],
            notes: "Uses hardened autoindex crawler for full recursive traversal and size mapping.",
        },
        AdapterSupportInfo {
            id: "nu_server",
            name: "Nu Server",
            support_level: "Full Crawl",
            matching_strategy: "Response preamble signature matching",
            sample_urls: vec!["http://nu-server.onion"],
            tested_for: vec![
                "Adapter fingerprint match (engine_test)",
                "Autoindex traversal delegation (nu adapter)",
            ],
            notes: "Delegates crawl execution to hardened autoindex traversal for directory/file extraction.",
        },
        AdapterSupportInfo {
            id: "inc_ransom",
            name: "INC Ransom Crawler",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain and blog signature matching",
            sample_urls: vec![
                "http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/698d5c538f1d14b7436dd63b",
            ],
            tested_for: vec!["Adapter fingerprint match (engine_test)"],
            notes: "Production adapter using disclosure API enrichment and crawl streaming.",
        },
        AdapterSupportInfo {
            id: "pear",
            name: "Pear Ransomware Crawler",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain and body signature matching",
            sample_urls: vec![
                "http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/",
            ],
            tested_for: vec!["Adapter fingerprint match (engine_test)"],
            notes: "Production adapter with concurrent crawl workers and UI batching.",
        },
        AdapterSupportInfo {
            id: "play",
            name: "Play Ransomware (Autoindex)",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain, URL-path, and autoindex fingerprint matching",
            sample_urls: vec![
                "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp",
            ],
            tested_for: vec![
                "Adapter fingerprint suite (engine_test + play_e2e_test)",
                "Feature and resilience suite (play_features_test)",
            ],
            notes: "Most heavily tested adapter with full listing/scaffold validation.",
        },
        AdapterSupportInfo {
            id: "abyss",
            name: "Abyss Ransomware",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain + direct archive URL detection",
            sample_urls: vec![
                "http://vmmefm7ktazj2bwtmy46o3wxhk42tctasyyqv6ymuzlivszteyhkkyad.onion/iamdesign.rar",
            ],
            tested_for: vec!["Adapter fingerprint match (engine_test)"],
            notes: "Handles direct archive downloads and directory listing traversal for Abyss leak sites.",
        },
        AdapterSupportInfo {
            id: "alphalocker",
            name: "AlphaLocker Ransomware",
            support_level: "Full Crawl",
            matching_strategy: "Known-domain and URL-path signature matching",
            sample_urls: vec![
                "http://3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad.onion/gazomet.pl%20&%20cgas.pl/Files/",
            ],
            tested_for: vec!["Adapter fingerprint match (engine_test)"],
            notes: "Handles autoindex and custom table-based listings with URL-encoded paths for AlphaLocker sites.",
        },
        AdapterSupportInfo {
            id: "autoindex",
            name: "Generic Autoindex",
            support_level: "Fallback",
            matching_strategy: "Generic 'Index of /' autoindex detection",
            sample_urls: vec!["http://unknown.onion/files/"],
            tested_for: vec!["Fallback adapter match (engine_test)"],
            notes: "Default catch-all fallback when specialized adapters do not match.",
        },
        AdapterSupportInfo {
            id: "universal_explorer",
            name: "Adaptive Universal Explorer",
            support_level: "Tier-4 Intelligent Fallback",
            matching_strategy: "Accepts all unmatched fingerprints — heuristic link scoring + speculative JoinSet prefetch",
            sample_urls: vec!["any unrecognized .onion URL"],
            tested_for: vec!["Compilation verification (cargo check --lib)"],
            notes: "Intelligent fallback that discovers structure on-the-fly via BinaryHeap scored traversal and learns winning paths in the TargetLedger.",
        },
    ]
}

pub struct AdapterRegistry {
    adapters: Vec<(String, Box<dyn CrawlerAdapter>)>,
    domain_cache: std::collections::HashMap<String, String>,
    regex_set: regex::RegexSet,
    regex_adapter_map: Vec<String>,
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::AdapterRegistry;

    #[test]
    fn url_hint_routes_qilin_cms_launcher() {
        let registry = AdapterRegistry::new();

        let view_adapter = registry
            .determine_adapter_from_url_hint(
                "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43",
            )
            .expect("view URL should resolve to Qilin");
        assert_eq!(view_adapter.name(), "Qilin Nginx Autoindex / CMS");

        let data_adapter = registry
            .determine_adapter_from_url_hint(
                "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/data?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43",
            )
            .expect("data URL should resolve to Qilin");
        assert_eq!(data_adapter.name(), "Qilin Nginx Autoindex / CMS");
    }
}

impl AdapterRegistry {
    fn adapter_by_id(&self, adapter_id: &str) -> Option<&dyn CrawlerAdapter> {
        self.adapters
            .iter()
            .find(|(id, _)| id == adapter_id)
            .map(|(_, adapter)| adapter.as_ref())
    }

    pub fn new() -> Self {
        Self::with_plugin_dir(None)
    }

    pub fn with_explorer_context(
        mut self,
        ledger: std::sync::Arc<crate::target_state::TargetLedger>,
    ) -> Self {
        self.adapters.push((
            "universal_explorer".to_string(),
            Box::new(universal_explorer::AdaptiveUniversalExplorer::new(ledger)),
        ));
        self
    }

    pub fn with_plugin_dir(plugin_dir: Option<&std::path::Path>) -> Self {
        let mut registry = AdapterRegistry {
            adapters: Vec::new(),
            domain_cache: std::collections::HashMap::new(),
            regex_set: regex::RegexSet::empty(),
            regex_adapter_map: Vec::new(),
        };

        let cache_path = std::path::Path::new("tests").join("known_domains.json");
        if let Ok(data) = std::fs::read_to_string(&cache_path) {
            if let Ok(parsed) = serde_json::from_str(&data) {
                registry.domain_cache = parsed;
            }
        }

        // Register all adapters — specific ones first, generic fallback last
        registry.adapters.push((
            "worldleaks".to_string(),
            Box::new(worldleaks::WorldLeaksAdapter),
        ));
        registry.adapters.push((
            "dragonforce".to_string(),
            Box::new(dragonforce::DragonForceAdapter),
        ));
        registry.adapters.push((
            "inc_ransom".to_string(),
            Box::new(inc_ransom::IncRansomAdapter),
        ));
        registry
            .adapters
            .push(("pear".to_string(), Box::new(pear::PearAdapter)));
        registry
            .adapters
            .push(("play".to_string(), Box::new(play::PlayAdapter)));
        registry
            .adapters
            .push(("lockbit".to_string(), Box::new(lockbit::LockBitAdapter)));
        registry
            .adapters
            .push(("abyss".to_string(), Box::new(abyss::AbyssAdapter)));
        registry.adapters.push((
            "alphalocker".to_string(),
            Box::new(alphalocker::AlphaLockerAdapter),
        ));
        registry
            .adapters
            .push(("qilin".to_string(), Box::new(qilin::QilinAdapter)));
        registry
            .adapters
            .push(("nu_server".to_string(), Box::new(nu::NuServerAdapter)));
        // FIX C-1: Old explorer.rs de-registered — universal_explorer.rs replaces it via with_explorer_context()

        for entry in plugin_host::load_runtime_plugins(plugin_dir, &mut registry.domain_cache) {
            registry.adapters.push(entry);
        }

        registry // Error placeholder if lines changed again
            .adapters
            .push((
                "autoindex".to_string(),
                Box::new(autoindex::AutoindexAdapter),
            )); // Generic fallback — always last

        // Precompile RegexSet Engine securely for the Tier 2 Bouncer
        let mut regex_patterns = Vec::new();
        let mut regex_adapter_map = Vec::new();
        for (id, adapter) in &registry.adapters {
            if let Some(pattern) = adapter.regex_marker() {
                regex_patterns.push(pattern);
                regex_adapter_map.push(id.clone());
            }
        }
        registry.regex_set =
            regex::RegexSet::new(&regex_patterns).unwrap_or_else(|_| regex::RegexSet::empty());
        registry.regex_adapter_map = regex_adapter_map;

        // Phase 117: Wire up known_domains() → domain_cache so adapters with
        // registered domains get instant O(1) matching without a JSON file.
        for (id, adapter) in &registry.adapters {
            for domain in adapter.known_domains() {
                registry
                    .domain_cache
                    .entry(domain.to_string())
                    .or_insert_with(|| id.clone());
            }
        }

        registry
    }

    pub async fn determine_adapter(
        &self,
        fingerprint: &SiteFingerprint,
    ) -> Option<&dyn CrawlerAdapter> {
        // 1. FAST PATH (M.A.C Tier 1): Check O(1) known domain database mapped to the specific adapter
        if let Ok(parsed_url) = reqwest::Url::parse(&fingerprint.url) {
            if let Some(domain) = parsed_url.domain() {
                if let Some(adapter_id) = self.domain_cache.get(domain) {
                    return self.adapter_by_id(adapter_id);
                }
            }
        }

        // 2. TIER 2 M.A.C: RegexSet Bouncer Pre-Filtering
        // The regex engine tests the entire 5MB HTML body once instantly in C-Speed.
        let matches: Vec<_> = self
            .regex_set
            .matches(&fingerprint.body)
            .into_iter()
            .collect();
        let mut candidates_to_check: Vec<&Box<dyn CrawlerAdapter>> = Vec::new();

        if !matches.is_empty() {
            // A specific Regex matched! We immediately know which heavy AST adapters to trigger.
            for match_idx in matches {
                let adapter_id = &self.regex_adapter_map[match_idx];
                for (id, adapter) in &self.adapters {
                    if id == adapter_id {
                        candidates_to_check.push(adapter);
                    }
                }
            }
        } else {
            // 3. TIER 3 M.A.C: Fallback to generic adapters that possess no specific signature.
            for (_, adapter) in &self.adapters {
                if adapter.regex_marker().is_none() {
                    candidates_to_check.push(adapter);
                }
            }
        }

        // FIX C-2: Sequential ordered iteration instead of FuturesUnordered race.
        // This guarantees specialized adapters (registered first) are always tried
        // before catch-all fallbacks like universal_explorer.
        for adapter in candidates_to_check {
            if adapter.can_handle(fingerprint).await {
                return Some(adapter.as_ref());
            }
        }

        None
    }

    pub fn determine_adapter_from_url_hint(&self, url: &str) -> Option<&dyn CrawlerAdapter> {
        let parsed_url = reqwest::Url::parse(url).ok()?;

        if let Some(domain) = parsed_url.domain() {
            if let Some(adapter_id) = self.domain_cache.get(domain) {
                return self.adapter_by_id(adapter_id);
            }
        }

        let path = parsed_url.path();
        let query = parsed_url.query().unwrap_or_default();
        let has_uuid_query = query.contains("uuid=");

        if has_uuid_query && (path.contains("/site/view") || path.contains("/site/data")) {
            return self.adapter_by_id("qilin");
        }

        None
    }
}
