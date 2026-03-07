use crate::adapters::autoindex::AutoindexAdapter;
use crate::adapters::{CrawlerAdapter, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::AppHandle;

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum PluginPipeline {
    #[default]
    Autoindex,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HeaderContainsRule {
    pub name: String,
    pub value_substring: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeAdapterPluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    host_pipeline: PluginPipeline,
    #[serde(default)]
    pub known_domains: Vec<String>,
    #[serde(default)]
    pub url_contains_any: Vec<String>,
    #[serde(default)]
    pub url_prefixes_any: Vec<String>,
    #[serde(default)]
    pub body_contains_all: Vec<String>,
    #[serde(default)]
    pub header_contains_all: Vec<HeaderContainsRule>,
    #[serde(default)]
    pub regex_marker: Option<String>,
}

#[derive(Clone, Debug)]
struct StaticHeaderContainsRule {
    name: &'static str,
    value_substring: &'static str,
}

pub struct PluginHostAdapter {
    id: &'static str,
    name: &'static str,
    pipeline: PluginPipeline,
    known_domains: Vec<&'static str>,
    url_contains_any: Vec<&'static str>,
    url_prefixes_any: Vec<&'static str>,
    body_contains_all: Vec<&'static str>,
    header_contains_all: Vec<StaticHeaderContainsRule>,
    regex_marker: Option<&'static str>,
}

impl PluginHostAdapter {
    fn from_manifest(manifest: RuntimeAdapterPluginManifest) -> Option<Self> {
        if manifest.id.trim().is_empty() || manifest.name.trim().is_empty() {
            return None;
        }

        Some(Self {
            id: leak_string(manifest.id),
            name: leak_string(manifest.name),
            pipeline: manifest.host_pipeline,
            known_domains: manifest
                .known_domains
                .into_iter()
                .map(normalize_domain)
                .map(leak_string)
                .collect(),
            url_contains_any: manifest
                .url_contains_any
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .map(leak_string)
                .collect(),
            url_prefixes_any: manifest
                .url_prefixes_any
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .map(leak_string)
                .collect(),
            body_contains_all: manifest
                .body_contains_all
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .map(leak_string)
                .collect(),
            header_contains_all: manifest
                .header_contains_all
                .into_iter()
                .filter(|rule| {
                    !rule.name.trim().is_empty() && !rule.value_substring.trim().is_empty()
                })
                .map(|rule| StaticHeaderContainsRule {
                    name: leak_string(rule.name),
                    value_substring: leak_string(rule.value_substring),
                })
                .collect(),
            regex_marker: manifest
                .regex_marker
                .filter(|value| !value.trim().is_empty())
                .map(leak_string),
        })
    }
}

#[async_trait::async_trait]
impl CrawlerAdapter for PluginHostAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        if !self.known_domains.is_empty() {
            let domain_matches = reqwest::Url::parse(&fingerprint.url)
                .ok()
                .and_then(|url| url.domain().map(str::to_ascii_lowercase))
                .map(|domain| self.known_domains.iter().any(|known| *known == domain))
                .unwrap_or(false);
            if !domain_matches {
                return false;
            }
        }

        if !self.url_prefixes_any.is_empty()
            && !self
                .url_prefixes_any
                .iter()
                .any(|prefix| fingerprint.url.starts_with(prefix))
        {
            return false;
        }

        if !self.url_contains_any.is_empty()
            && !self
                .url_contains_any
                .iter()
                .any(|needle| fingerprint.url.contains(needle))
        {
            return false;
        }

        if !self
            .body_contains_all
            .iter()
            .all(|needle| fingerprint.body.contains(needle))
        {
            return false;
        }

        for rule in &self.header_contains_all {
            let Some(value) = fingerprint.headers.get(rule.name) else {
                return false;
            };
            let Ok(value_str) = value.to_str() else {
                return false;
            };
            if !value_str.contains(rule.value_substring) {
                return false;
            }
        }

        true
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        match self.pipeline {
            PluginPipeline::Autoindex => {
                <AutoindexAdapter as CrawlerAdapter>::crawl(
                    &AutoindexAdapter,
                    current_url,
                    frontier,
                    app,
                )
                .await
            }
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn known_domains(&self) -> Vec<&'static str> {
        self.known_domains.clone()
    }

    fn regex_marker(&self) -> Option<&'static str> {
        self.regex_marker
    }
}

pub fn load_runtime_plugins(
    plugin_dir_override: Option<&Path>,
    domain_cache: &mut HashMap<String, String>,
) -> Vec<(String, Box<dyn CrawlerAdapter>)> {
    let Some(plugin_dir) = discover_plugin_dir(plugin_dir_override) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(&plugin_dir) else {
        return entries;
    };

    let mut manifest_paths: Vec<PathBuf> = read_dir
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    manifest_paths.sort();

    for manifest_path in manifest_paths {
        let Ok(content) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<RuntimeAdapterPluginManifest>(&content) else {
            continue;
        };
        let Some(adapter) = PluginHostAdapter::from_manifest(manifest.clone()) else {
            continue;
        };

        let plugin_id = adapter.id.to_string();
        for domain in &manifest.known_domains {
            let normalized = normalize_domain(domain);
            if !normalized.is_empty() {
                domain_cache.insert(normalized, plugin_id.clone());
            }
        }

        entries.push((plugin_id, Box::new(adapter) as Box<dyn CrawlerAdapter>));
    }

    entries
}

fn discover_plugin_dir(plugin_dir_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = plugin_dir_override {
        return dir.exists().then(|| dir.to_path_buf());
    }

    if let Ok(dir) = std::env::var("CRAWLI_ADAPTER_PLUGIN_DIR") {
        let path = PathBuf::from(dir);
        if path.exists() {
            return Some(path);
        }
    }

    let Ok(cwd) = std::env::current_dir() else {
        return None;
    };

    [
        cwd.join("adapter_plugins"),
        cwd.join("plugins").join("adapters"),
        cwd.join("../adapter_plugins"),
        cwd.join("../plugins").join("adapters"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn normalize_domain(value: impl AsRef<str>) -> String {
    let raw = value.as_ref().trim();
    if raw.is_empty() {
        return String::new();
    }

    reqwest::Url::parse(raw)
        .ok()
        .and_then(|url| url.domain().map(str::to_ascii_lowercase))
        .or_else(|| {
            let trimmed = raw
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/');
            (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
        })
        .unwrap_or_default()
}

fn leak_string(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::{normalize_domain, RuntimeAdapterPluginManifest};

    #[test]
    fn normalizes_manifest_domains() {
        assert_eq!(
            normalize_domain("https://example.onion/path"),
            "example.onion"
        );
        assert_eq!(normalize_domain("example.onion/"), "example.onion");
    }

    #[test]
    fn manifest_deserializes() {
        let manifest: RuntimeAdapterPluginManifest = serde_json::from_str(
            r#"{
                "id":"fixture",
                "name":"Fixture Plugin",
                "host_pipeline":"autoindex",
                "known_domains":["fixture.onion"],
                "body_contains_all":["Index of /fixture/"]
            }"#,
        )
        .expect("manifest should deserialize");
        assert_eq!(manifest.id, "fixture");
        assert_eq!(manifest.name, "Fixture Plugin");
        assert_eq!(manifest.known_domains, vec!["fixture.onion"]);
    }
}
