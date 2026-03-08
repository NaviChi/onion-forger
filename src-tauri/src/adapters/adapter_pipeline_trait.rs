use anyhow::Result;
use rhai::{Dynamic, Engine, Scope};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlContext {
    pub target_url: String,
    pub raw_html: String,
    pub current_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: Option<u64>,
    pub date: Option<String>,
    pub url: String,
    pub is_dir: bool,
}

/// The core extensibility trait for Crawli.
/// This allows dynamic loading of scraping logic without recompilation.
pub trait CrawlAdapter: Send + Sync {
    /// Adapter identifier (e.g., "qilin_v2", "lockbit_v3")
    fn id(&self) -> &str;

    /// Parses an HTML buffer and extracts file and directory entries.
    fn parse_directory_html(&self, context: &CrawlContext) -> Result<Vec<FileEntry>>;
}

/// A lightweight, scriptable adapter that executes Rhai scripts
/// (4GB VM friendly compared to full Wasmtime/Wasmer).
pub struct RhaiScriptAdapter {
    id: String,
    script_content: String,
    engine: Engine,
}

impl RhaiScriptAdapter {
    pub fn new(id: impl Into<String>, script_content: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            script_content: script_content.into(),
            engine: Engine::new(),
        }
    }
}

impl CrawlAdapter for RhaiScriptAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn parse_directory_html(&self, context: &CrawlContext) -> Result<Vec<FileEntry>> {
        let mut scope = Scope::new();
        let mut ctx_map = rhai::Map::new();
        ctx_map.insert(
            "target_url".into(),
            Dynamic::from(context.target_url.clone()),
        );
        ctx_map.insert("raw_html".into(), Dynamic::from(context.raw_html.clone()));
        ctx_map.insert(
            "current_depth".into(),
            Dynamic::from(context.current_depth as i64),
        );

        scope.push("ctx", ctx_map);

        // The script returns an array of maps
        let dynamic_result: rhai::Array = self
            .engine
            .eval_with_scope(&mut scope, &self.script_content)
            .map_err(|e| anyhow::anyhow!("Adapter '{}' script error: {}", self.id, e))?;

        let mut entries = Vec::with_capacity(dynamic_result.len());

        for item in dynamic_result {
            if let Some(map) = item.try_cast::<rhai::Map>() {
                if let (Some(name_dyn), Some(url_dyn), Some(is_dir_dyn)) =
                    (map.get("name"), map.get("url"), map.get("is_dir"))
                {
                    let name = name_dyn.to_string();
                    let url = url_dyn.to_string();
                    let is_dir = is_dir_dyn.as_bool().unwrap_or(false);

                    let size = map
                        .get("size")
                        .and_then(|v| v.as_int().ok())
                        .map(|s| s as u64);
                    let date = map.get("date").map(|v| v.to_string());

                    entries.push(FileEntry {
                        name,
                        size,
                        date,
                        url,
                        is_dir,
                    });
                }
            }
        }

        Ok(entries)
    }
}
