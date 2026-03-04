use tauri::AppHandle;

#[derive(Default)]
pub struct LockBitAdapter;

#[async_trait::async_trait]
impl super::CrawlerAdapter for LockBitAdapter {
    async fn can_handle(&self, fingerprint: &super::SiteFingerprint) -> bool {
        // We check for LockBit specific markers like the autoindex start comment
        fingerprint.url.to_ascii_lowercase().contains("lockbit")
            || fingerprint.body.contains("<!-- Start of nginx output -->")
            || fingerprint.body.to_ascii_lowercase().contains("lockbit")
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: std::sync::Arc<crate::frontier::CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<super::FileEntry>> {
        // LockBit surfaces commonly expose nginx/autoindex style directory trees.
        // Reuse the hardened generic autoindex crawler instead of returning an empty set.
        <crate::adapters::autoindex::AutoindexAdapter as super::CrawlerAdapter>::crawl(
            &crate::adapters::autoindex::AutoindexAdapter,
            current_url,
            frontier,
            app,
        )
        .await
    }

    fn name(&self) -> &'static str {
        "LockBit Embedded Nginx"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        // LockBit hosts rotate; keep a resilient host-token fast-path.
        vec!["lockbit"]
    }
}
