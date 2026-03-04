use tauri::AppHandle;

#[derive(Default)]
pub struct QilinAdapter;

#[async_trait::async_trait]
impl super::CrawlerAdapter for QilinAdapter {
    async fn can_handle(&self, fingerprint: &super::SiteFingerprint) -> bool {
        // Qilin uses a themed autoindex but lacks standard "Index of /" headers.
        fingerprint.body.contains("<div class=\"page-header-title\">QData</div>")
            || fingerprint.body.contains("Data browser")
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: std::sync::Arc<crate::frontier::CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<super::FileEntry>> {
        // Qilin utilizes a customized CSS Nginx/Apache autoindex layout.
        // The table layout (`<table id="list">`) and child traversal is perfectly compliant with the standard autoindex parser.
        <crate::adapters::autoindex::AutoindexAdapter as super::CrawlerAdapter>::crawl(
            &crate::adapters::autoindex::AutoindexAdapter,
            current_url,
            frontier,
            app,
        )
        .await
    }

    fn name(&self) -> &'static str {
        "Qilin Nginx Autoindex"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        // Known Qilin root onions.
        // It provides direct O(1) matching without HTML body probes.
        vec![
            "iv6lrjrd5ioyanvvemnkhturmyfpfbdcy442e22oqd2izkwnjw23m3id.onion",
            "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion",
        ]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r#"<div class="page-header-title">QData</div>|Data browser"#)
    }
}
