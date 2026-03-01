use tauri::AppHandle;

#[derive(Default)]
pub struct NuServerAdapter;

#[async_trait::async_trait]
impl super::CrawlerAdapter for NuServerAdapter {
    async fn can_handle(&self, fingerprint: &super::SiteFingerprint) -> bool {
        // Identify by response starting with "# acct" or "# srvinf: nu"
        fingerprint.body.starts_with("# acct") || fingerprint.body.starts_with("# srvinf: nu")
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
        "Nu Server"
    }
}
