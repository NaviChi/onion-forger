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
        current_url: &str,
        frontier: std::sync::Arc<crate::frontier::CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<super::FileEntry>> {
        // Nu servers often expose lightweight index structures compatible with the
        // generic autoindex traversal strategy.
        <crate::adapters::autoindex::AutoindexAdapter as super::CrawlerAdapter>::crawl(
            &crate::adapters::autoindex::AutoindexAdapter,
            current_url,
            frontier,
            app,
        )
        .await
    }

    fn name(&self) -> &'static str {
        "Nu Server"
    }
}
