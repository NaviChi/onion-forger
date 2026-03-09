// src-tauri/src/adapters/universal_explorer.rs
// Adaptive Universal Explorer - Tier-4 Intelligent Fallback
// Features: Assassin JoinSet prefetch, target ledger learning, domain-bounded scoring

use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::target_state::TargetLedger;
use scraper::{Html, Selector};
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use tauri::AppHandle;
use url::Url;

#[derive(Eq, PartialEq)]
struct ScoredLink {
    score: i32,
    url: String,
    depth: u32,
}

impl Ord for ScoredLink {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score.cmp(&other.score) // max-heap: highest score pops first (FIX M-1)
    }
}
impl PartialOrd for ScoredLink {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct AdaptiveUniversalExplorer {
    ledger: Arc<TargetLedger>,
}

impl AdaptiveUniversalExplorer {
    pub fn new(ledger: Arc<TargetLedger>) -> Self {
        Self { ledger }
    }

    fn calculate_link_score(&self, url: &Url, root: &str, text: &str, depth: u32) -> i32 {
        let mut score = 50;
        let root_host = Url::parse(root)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()));
        let link_host = url.host_str().map(|h| h.to_string());
        if link_host == root_host {
            score += 50;
        }
        if url.path().contains("file")
            || url.path().contains("data")
            || url.path().contains("storage")
            || url.path().contains("archive")
        {
            score += 40;
        }
        let text_lower = text.to_lowercase();
        if text_lower.contains("download") || text_lower.contains("file") {
            score += 30;
        }
        if url.path().ends_with(".zip")
            || url.path().ends_with(".rar")
            || url.path().ends_with(".7z")
            || url.path().ends_with(".sql")
            || url.path().ends_with(".db")
        {
            score += 50;
        }
        if depth > 4 {
            score -= 20;
        }
        score
    }

    /// Parse page using raw HTML body directly.
    /// Incorporates Tier-4 Adaptive Hydrator Wire Mode Detection (SPA JSON vs Autoindex)
    pub fn parse_page_from_body(&self, body: &str, base_url: &str, app: Option<&AppHandle>) -> Option<Vec<FileEntry>> {
        use tauri::Emitter;
        let base_parsed_url =
            Url::parse(base_url).unwrap_or_else(|_| Url::parse("http://unknown.onion").unwrap());
        let host = base_parsed_url.host_str().unwrap_or("unknown.onion");

        // Tier-4 Adaptive Hydrator: Mode 1 - NextJS SPA / Predictive State Mimicry
        let has_next = body.contains("__NEXT_DATA__");
        let has_fsguest = body.contains("fsguest");
        let has_token = body.contains("token=");

        if has_next || has_fsguest || has_token {
            if let Some(app) = app {
                let indicators = [
                    if has_next { Some("__NEXT_DATA__") } else { None },
                    if has_fsguest { Some("fsguest") } else { None },
                    if has_token { Some("token=") } else { None },
                ].into_iter().flatten().collect::<Vec<_>>().join(", ");
                
                let _ = app.emit("crawl_log", format!("[🤖 Tier-4 Hydrator] SPA indicators detected ({}) on target: {}. Abandoning Autoindex for SPA JSON Mode 1 (Mimicry).", indicators, base_url));
            }
            let spa_entries = crate::adapters::dragonforce::parse_dragonforce_fsguest(body, host, base_url);
            if !spa_entries.is_empty() {
                if let Some(app) = app {
                    let _ = app.emit("crawl_log", format!("[🤖 Tier-4 Hydrator] Successfully extrapolated {} API routes bridging JS to raw state", spa_entries.len()));
                }
                return Some(spa_entries);
            }
            if let Some(app) = app {
                let _ = app.emit("crawl_log", "[🤖 Tier-4 Hydrator] Mode 1 active but extraction yielded 0 routes. Falling back...".to_string());
            }
        }

        // Tier-4 Adaptive Hydrator: Mode 2 - CMS UUID / Iframe Embed (Mimics LockBit/DragonForce hybrids)
        if body.contains("<iframe") && body.contains("src=") {
            if let Some(app) = app {
                let _ = app.emit("crawl_log", format!("[🤖 Tier-4 Hydrator] Iframe proxy indicators detected (<iframe src=). Abandoning Autoindex for SPA JSON Mode 2 on target: {}", base_url));
            }
            let spa_entries = crate::adapters::dragonforce::parse_dragonforce_fsguest(body, host, base_url);
            if !spa_entries.is_empty() {
                if let Some(app) = app {
                    let _ = app.emit("crawl_log", format!("[🤖 Tier-4 Hydrator] Extracted {} nested entries from Iframe embed", spa_entries.len()));
                }
                return Some(spa_entries);
            }
        }

        // Tier-4 Adaptive Hydrator: Mode 3 - Generic Autoindex
        let parsed = crate::adapters::autoindex::parse_autoindex_html(body);

        if !parsed.is_empty() {
            if let Some(app) = app {
                let _ = app.emit("crawl_log", format!("[🤖 Tier-4 Hydrator] Mode 3 (Classic Autoindex) matched {} items on: {}", parsed.len(), base_url));
            }
            let mut entries = Vec::new();
            for (name, size, is_dir) in parsed {
                if let Ok(full) = base_parsed_url.join(&name) {
                    entries.push(FileEntry {
                        jwt_exp: None,
                        path: full.path().to_string(),
                        size_bytes: size,
                        entry_type: if is_dir {
                            EntryType::Folder
                        } else {
                            EntryType::File
                        },
                        raw_url: full.to_string(),
                    });
                }
            }
            return Some(entries);
        }

        None
    }
}

#[async_trait::async_trait]
impl CrawlerAdapter for AdaptiveUniversalExplorer {
    async fn can_handle(&self, _fingerprint: &SiteFingerprint) -> bool {
        true // ultimate fallback — only reached after all specialized adapters decline
    }

    fn name(&self) -> &'static str {
        "Universal Explorer v2"
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;

        let mut visited = std::collections::HashSet::new();
        // FIX M-1: plain max-heap (no Reverse) — highest score pops first
        let mut queue: BinaryHeap<ScoredLink> = BinaryHeap::new();
        queue.push(ScoredLink {
            score: 100,
            url: current_url.to_string(),
            depth: 0,
        });

        let mut results = Vec::new();
        let max_depth = 5;
        let max_prefetch = 6; // governor-aware cap (PR-EXPLORER-001)

        // FIX H-4: Response cache so prefetched pages aren't fetched twice
        let mut body_cache: HashMap<String, String> = HashMap::new();

        // FIX H-1: Extract root host for domain boundary enforcement
        let root_host = Url::parse(current_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()));

        while let Some(link) = queue.pop() {
            if link.depth > max_depth || visited.len() > 10000 || frontier.is_cancelled() {
                break;
            }
            if !visited.insert(link.url.clone()) {
                continue;
            }

            // FIX M-3: synchronous prefix boost (no async needed)
            let boost = self.ledger.get_learned_prefix_boost(&link.url);

            // FIX M-5: Emit UI progress events
            if visited.len() % 5 == 0 || visited.len() == 1 {
                let _ = app.emit(
                    "crawl_log",
                    format!(
                        "[Explorer] depth={} score={} pages={} files={} — {}",
                        link.depth,
                        link.score + boost,
                        visited.len(),
                        results.len(),
                        if link.url.len() > 80 {
                            &link.url[..80]
                        } else {
                            &link.url
                        }
                    ),
                );
            }

            let (cid1, client1) = frontier.get_client();
            let (cid2, client2) = frontier.get_client();

            // FIX H-4: Check body cache before fetching
            // Phase 73: Speculative Dual-Circuit Tor GET Racing
            let body = if let Some(cached) = body_cache.remove(&link.url) {
                Some(cached)
            } else {
                let start = std::time::Instant::now();
                let link_clone1 = link.url.clone();
                let link_clone2 = link.url.clone();
                
                let req1 = Box::pin(async {
                    let res = client1.get(&link_clone1).send().await;
                    (cid1, res)
                });
                
                let req2 = Box::pin(async {
                    let res = client2.get(&link_clone2).send().await;
                    (cid2, res)
                });
                
                let (winner_cid, fetch_result) = match futures::future::select(req1, req2).await {
                    futures::future::Either::Left((res, _)) => res,
                    futures::future::Either::Right((res, _)) => res,
                };

                match fetch_result {
                    Ok(resp) => {
                        let text = resp.text().await.ok();
                        let len = text.as_ref().map(|t| t.len() as u64).unwrap_or(0);
                        frontier.record_success(winner_cid, len, start.elapsed().as_millis() as u64);
                        text
                    }
                    Err(_) => {
                        frontier.record_failure(winner_cid);
                        None
                    }
                }
            };

            // Collect entries and children synchronously in a block so
            // `scraper::Html` (which is !Send) is dropped before any JoinSet .await.
            let mut children_for_prefetch: Vec<String> = Vec::new(); // FIX M-2: just URLs
            if let Some(body) = body {
                // FIX M-4: Parse directly from body string, not from DOM re-serialize
                if let Some(entries) = self.parse_page_from_body(&body, &link.url, Some(&app)) {
                    results.extend(entries);
                }

                // Phase 73: HFT DOM Deserialization & Pre-Heating over spawn_blocking
                let raw_children = {
                    let body_clone = body.clone();
                    let link_url_clone = link.url.clone();
                    let root_host_clone = root_host.clone();
                    
                    tokio::task::spawn_blocking(move || {
                        let document = Html::parse_document(&body_clone);
                        let selector = Selector::parse("a[href]").unwrap();
                        let mut raw = Vec::new();
                        for element in document.select(&selector) {
                            if let Some(href) = element.value().attr("href") {
                                if let Ok(full) = Url::parse(&link_url_clone).and_then(|u| u.join(href)) {
                                    // FIX H-1: Hard domain boundary — reject off-host links
                                    let link_host = full.host_str().map(|h| h.to_string());
                                    if link_host != root_host_clone {
                                        continue;
                                    }
                                    let text: String = element.text().collect();
                                    raw.push((full.to_string(), text));
                                }
                            }
                        }
                        raw
                    }).await.unwrap_or_default()
                };

                // Score and deduplicate children
                let mut scored_children = Vec::new();
                for (full_url, text) in raw_children {
                    if frontier.mark_visited(&full_url) {
                        if let Ok(parsed) = Url::parse(&full_url) {
                            let score =
                                self.calculate_link_score(&parsed, current_url, &text, link.depth);
                            scored_children.push(ScoredLink {
                                score,
                                url: full_url,
                                depth: link.depth + 1,
                            });
                        }
                    }
                }

                // Sort children by highest score first for prefetch selection
                scored_children.sort_by(|a, b| b.score.cmp(&a.score));

                // Collect top N for parallel prefetch, push all into queue
                for child in scored_children.into_iter().take(max_prefetch) {
                    children_for_prefetch.push(child.url.clone());
                    queue.push(child);
                }
            }

            // FIX H-4: Speculative prefetch — cache response bodies instead of discarding
            if !children_for_prefetch.is_empty() {
                let mut join_set = tokio::task::JoinSet::new();
                for child_url in children_for_prefetch {
                    let (pcid, warm_client) = frontier.get_client();
                    let frontier_ref = frontier.clone();
                    join_set.spawn(async move {
                        let start = std::time::Instant::now();
                        match warm_client.get(&child_url).send().await {
                            Ok(resp) => {
                                let text = resp.text().await.unwrap_or_default();
                                frontier_ref.record_success(
                                    pcid,
                                    text.len() as u64,
                                    start.elapsed().as_millis() as u64,
                                );
                                Some((child_url, text))
                            }
                            Err(_) => {
                                frontier_ref.record_failure(pcid);
                                None
                            }
                        }
                    });
                }
                // Collect ALL prefetch results into cache (not just first)
                while let Some(res) = join_set.join_next().await {
                    if let Ok(Some((url, body))) = res {
                        body_cache.insert(url, body);
                    }
                }
            }
        }

        let _ = app.emit(
            "crawl_log",
            format!(
                "[Explorer] Complete — {} files discovered across {} pages",
                results.len(),
                visited.len()
            ),
        );

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_heap_ordering() {
        let mut heap = BinaryHeap::new();
        heap.push(ScoredLink {
            score: 10,
            url: "low".into(),
            depth: 0,
        });
        heap.push(ScoredLink {
            score: 90,
            url: "high".into(),
            depth: 0,
        });
        heap.push(ScoredLink {
            score: 50,
            url: "mid".into(),
            depth: 0,
        });

        // Max-heap: highest score pops first
        assert_eq!(heap.pop().unwrap().url, "high");
        assert_eq!(heap.pop().unwrap().url, "mid");
        assert_eq!(heap.pop().unwrap().url, "low");
    }

    #[test]
    fn test_domain_boundary_scoring() {
        let explorer = AdaptiveUniversalExplorer::new(Arc::new(TargetLedger::default()));

        let same_host = Url::parse("http://abc123.onion/files/test.zip").unwrap();
        let diff_host = Url::parse("http://evil999.onion/malware.exe").unwrap();

        let score_same =
            explorer.calculate_link_score(&same_host, "http://abc123.onion/", "download file", 0);
        let score_diff =
            explorer.calculate_link_score(&diff_host, "http://abc123.onion/", "download file", 0);

        // Same host gets +50 bonus, different host does not
        assert!(
            score_same > score_diff,
            "same_host={} should be > diff_host={}",
            score_same,
            score_diff
        );
        // Same host with zip extension + download text should be high
        assert!(
            score_same >= 180,
            "same_host score should be >= 180, got {}",
            score_same
        );
    }

    #[test]
    fn test_parse_page_from_body() {
        let explorer = AdaptiveUniversalExplorer::new(Arc::new(TargetLedger::default()));
        let html = r#"<html><body><h1>Index of /files/</h1>
<a href="data.zip">data.zip</a>   2024-01-01 12:00   1024
<a href="docs/">docs/</a>         2024-01-01 12:00    -
<a href="../">../</a>
</body></html>"#;

        let entries = explorer.parse_page_from_body(html, "http://test.onion/files/", None);
        assert!(entries.is_some(), "Should parse autoindex entries");
        let entries = entries.unwrap();
        assert_eq!(
            entries.len(),
            2,
            "Should find 2 entries (data.zip + docs/), not ../"
        );
        assert_eq!(entries[0].entry_type, EntryType::File);
        assert_eq!(entries[1].entry_type, EntryType::Folder);
    }

    #[test]
    fn test_learned_prefix_boost_sync() {
        let mut ledger = TargetLedger::default();
        ledger.learned_prefixes = vec!["http://abc.onion/known/".to_string()];

        // Matching prefix gets +1000
        assert_eq!(
            ledger.get_learned_prefix_boost("http://abc.onion/known/data.zip"),
            1000
        );
        // Non-matching prefix gets 0
        assert_eq!(
            ledger.get_learned_prefix_boost("http://abc.onion/unknown/data.zip"),
            0
        );
    }
}
