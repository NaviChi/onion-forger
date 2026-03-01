use tauri::AppHandle;

#[derive(Default)]
pub struct LockBitAdapter;

#[async_trait::async_trait]
impl super::CrawlerAdapter for LockBitAdapter {
    async fn can_handle(&self, fingerprint: &super::SiteFingerprint) -> bool {
        // We check for LockBit specific markers like the autoindex start comment
        fingerprint.body.contains("<!-- Start of nginx output -->") || fingerprint.body.contains("lockbit")
    }

    async fn crawl(
        &self, 
        _current_url: &str, 
        _frontier: std::sync::Arc<crate::frontier::CrawlerFrontier>, 
        _app: AppHandle
    ) -> anyhow::Result<Vec<super::FileEntry>> {
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "LockBit Embedded Nginx"
    }
}
