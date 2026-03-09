use crate::adapters::{CrawlerAdapter, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use std::sync::Arc;
use tauri::AppHandle;

/// Genesis Ransomware Adapter
///
/// New target provided by user: `http://genesis6ixpb5mcy4kudybtw5op2wqlrkocfogbnenz3c647ibqixiad.onion/download/cce17aec4f3cbc4d7db_part2.tar.gz`
#[derive(Default)]
pub struct GenesisAdapter;

#[async_trait::async_trait]
impl CrawlerAdapter for GenesisAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        let url_lower = fingerprint.url.to_ascii_lowercase();
        url_lower.contains("genesis6ixpb5mcy4kudybtw5op2wqlrkocfogbnenz3c647ibqixiad") 
            || url_lower.contains("genesis")
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        _app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        // Just fetch the root HTTP headers to see what Genesis sends back.
        let (cid, client) = frontier.get_client();
        if let Ok(Ok(resp)) = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            client.get(current_url).header("Range", "bytes=0-0").send(),
        )
        .await
        {
            let headers = format!("{:#?}", resp.headers());
            let dump_path = "/tmp/genesis_headers_dump.txt";
            let _ = std::fs::write(dump_path, &headers);
            println!("[GENESIS] Dumped headers to {}", dump_path);
        } else {
            println!("[GENESIS] Connection failed or timed out.");
            frontier.record_failure(cid);
        }

        // Return empty vector since we don't know the format yet
        Ok(Vec::new())
    }

    fn name(&self) -> &'static str {
        "Genesis Archive Direct"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["genesis6ixpb5mcy4kudybtw5op2wqlrkocfogbnenz3c647ibqixiad.onion"]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r"(?i)genesis")
    }
}
