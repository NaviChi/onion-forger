use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use std::collections::BinaryHeap;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct ExplorerAdapter;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScoredLink {
    score: i32,
    url: String,
    depth: u32,
}

impl PartialOrd for ScoredLink {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredLink {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score.cmp(&other.score)
    }
}

fn calculate_link_score(
    url: &reqwest::Url,
    _root: &str,
    text: &str,
    depth: u32,
    learned: &[String],
) -> i32 {
    let mut score = 50; // base for same-host

    let path_str = url.path();

    // Check Learned Prefixes First
    for prefix in learned {
        if path_str.starts_with(prefix) {
            // Give massive boost so known paths are probed before anything else.
            // Decay logic could inspect timestamp, but simple massive priority is safest here.
            score += 1000;
            break;
        }
    }

    if path_str.contains("file")
        || path_str.contains("data")
        || path_str.contains("storage")
        || path_str.contains("archive")
    {
        score += 40;
    }
    if text.to_lowercase().contains("download") || text.to_lowercase().contains("file") {
        score += 30;
    }
    if path_str.ends_with(".zip")
        || path_str.ends_with(".rar")
        || path_str.ends_with(".7z")
        || path_str.ends_with(".sql")
        || path_str.ends_with(".db")
    {
        score += 50; // strong file signal
    }
    if depth > 4 {
        score -= 20; // penalize deep recursion early
    }
    score
}

#[async_trait::async_trait]
impl CrawlerAdapter for ExplorerAdapter {
    async fn can_handle(&self, _fingerprint: &SiteFingerprint) -> bool {
        // Fallback for any unknown structure
        true
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        _app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        let (cid, client) = frontier.get_client();

        let req_start = std::time::Instant::now();
        let resp = client.get(current_url).send().await?;
        let final_url = resp.url().to_string();

        // Mode 3: CMS Redirect Mode handled by checking if reqwest natively followed a 302
        let _is_cms_redirect = final_url != current_url && final_url.contains("uuid=");

        let body = resp.text().await?;
        frontier.record_success(
            cid,
            body.len() as u64,
            req_start.elapsed().as_millis() as u64,
        );

        // Mode 1: Autoindex Mode Detection
        if body.contains("Index of /") || body.contains("<table") {
            // Switch to Autoindex parsing mode
            let mut entries = Vec::new();
            let parsed = crate::adapters::autoindex::parse_autoindex_html(&body);
            for (filename, size, is_dir) in parsed {
                let encoded = path_utils::url_encode(&filename);
                let child_url = format!("{}/{}", final_url.trim_end_matches('/'), encoded);
                entries.push(FileEntry {
                    jwt_exp: None,
                    path: format!("/{}", path_utils::sanitize_path(&filename)),
                    size_bytes: size,
                    entry_type: if is_dir {
                        EntryType::Folder
                    } else {
                        EntryType::File
                    },
                    raw_url: child_url,
                });
            }
            if !entries.is_empty() {
                return Ok(entries); // Fast return for flat tables
            }
        }

        // Mode 2: SPA / JSON Mode Detection
        if body.contains("__NEXT_DATA__") {
            // Switch to Next.js JSON Extraction Mode
            // (Scaffolded extraction)
        }

        if let Some(iframe_start) = body.find("iframe src=\"") {
            let rem = &body[iframe_start + 12..];
            if let Some(iframe_end) = rem.find("\"") {
                let iframe_url = &rem[..iframe_end];
                if iframe_url.contains("token=") {
                    // Navigate into authenticated iframe
                }
            }
        }

        // Load Persistent Ledger Intelligence for specific target bounds
        let mut learned = Vec::new();
        if let Some(paths) = frontier.target_paths() {
            if let Ok(l) = crate::target_state::load_or_default_ledger(paths) {
                learned = l.learned_prefixes.clone();
            }
        }

        // Fallback: Intelligent Priority Link Routing
        let queue = Arc::new(Mutex::new(BinaryHeap::new()));

        queue.lock().await.push(ScoredLink {
            score: 100,
            url: final_url.clone(), // Clone to avoid move violation
            depth: 0,
        });

        // The "Assassin JoinSet"
        // 1. Pop top 6 most probable structural links.
        // 2. Launch concurrently using rotating multi-client bounds.
        // 3. Immediately abort_all() the moment ANY task successfully identifies a structured directory tree!

        let mut speculative_joinset = tokio::task::JoinSet::new();

        // Extract heuristic links from root body
        let root_url = reqwest::Url::parse(&final_url)?;
        let mut temp_links = Vec::new();

        // 1. Synchronously parse DOM (scraper objects are NOT `Send`, cannot cross await boundaries)
        {
            let doc = scraper::Html::parse_document(&body);
            let selector = scraper::Selector::parse("a").unwrap();
            for element in doc.select(&selector) {
                if let Some(href) = element.value().attr("href") {
                    if let Ok(joined) = root_url.join(href) {
                        let text = element.text().collect::<Vec<_>>().join(" ");
                        let score = calculate_link_score(&joined, &final_url, &text, 1, &learned);
                        temp_links.push(ScoredLink {
                            score,
                            url: joined.to_string(),
                            depth: 1,
                        });
                    }
                }
            }
        }

        // 2. Safely push extracted links into the Async Mutex Queue
        let mut queue_lock = queue.lock().await;
        for link in temp_links {
            queue_lock.push(link);
        }

        // Drop lock before entering the joinset logic
        drop(queue_lock);

        // Pop the top 6 highest scored links for the Assassin Prefetch
        let max_probes = if std::env::var("CRAWLI_GOVERNOR_STRICT").is_ok() {
            4
        } else {
            8
        };

        let mut popped = Vec::new();
        for _ in 0..max_probes {
            if let Some(link) = queue.lock().await.pop() {
                popped.push(link.url);
            } else {
                break;
            }
        }

        if popped.is_empty() {
            return Ok(vec![]);
        }

        // Run the Assassin Multiplexer
        for url_to_probe in popped {
            let (cid, client) = frontier.get_client();
            let frontier_clone = frontier.clone();

            speculative_joinset.spawn(async move {
                let start = std::time::Instant::now();
                if let Ok(resp) = client.get(&url_to_probe).send().await {
                    let probe_final_url = resp.url().to_string();
                    if let Ok(text) = resp.text().await {
                        frontier_clone.record_success(
                            cid,
                            text.len() as u64,
                            start.elapsed().as_millis() as u64,
                        );
                        if text.contains("Index of /") || text.contains("<table") {
                            // We found a winning structure!
                            return Some((probe_final_url, text));
                        }
                    }
                }
                None
            });
        }

        while let Some(res) = speculative_joinset.join_next().await {
            match res {
                Ok(Some((winning_url, text))) => {
                    // Mission Accomplished: A probe found structure!
                    // Abort all other flights instantly to save Tor bandwidth and WAF reputation
                    speculative_joinset.abort_all();

                    // Add to Learned Prefixes in Ledger for next time
                    if let Some(paths) = frontier.target_paths() {
                        if let Ok(mut l) = crate::target_state::load_or_default_ledger(paths) {
                            if let Ok(parsed) = reqwest::Url::parse(&winning_url) {
                                let prefix = parsed.path().to_string();
                                if !l.learned_prefixes.contains(&prefix) {
                                    l.learned_prefixes.push(prefix);
                                    let _ = crate::target_state::save_ledger(paths, &l);
                                }
                            }
                        }
                    }

                    // Delegate out to Autoindex to extract the files
                    let mut entries = Vec::new();
                    let parsed = crate::adapters::autoindex::parse_autoindex_html(&text);
                    for (filename, size, is_dir) in parsed {
                        let encoded = path_utils::url_encode(&filename);
                        let child_url =
                            format!("{}/{}", winning_url.trim_end_matches('/'), encoded);
                        entries.push(FileEntry {
                            jwt_exp: None,
                            path: format!("/{}", path_utils::sanitize_path(&filename)),
                            size_bytes: size,
                            entry_type: if is_dir {
                                EntryType::Folder
                            } else {
                                EntryType::File
                            },
                            raw_url: child_url,
                        });
                    }
                    return Ok(entries);
                }
                _ => continue,
            }
        }

        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "Adaptive Universal Explorer"
    }
}
