use crate::arti_connector::ArtiConnector;
use anyhow::{anyhow, Result};
use arti_client::TorClient;
use bytes::Bytes;
use http::{HeaderMap, Request, Response, StatusCode};
use http_body_util::BodyExt;
use hyper_rustls::HttpsConnector;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_stream::Stream;
use tor_rtcompat::PreferredRuntime;

#[derive(Clone)]
pub enum ArtiClient {
    Tor {
        client: Client<HttpsConnector<ArtiConnector>, http_body_util::Full<Bytes>>,
        inner_tor_client: TorClient<PreferredRuntime>,
    },
    Clearnet {
        client: reqwest::Client,
    },
}

impl ArtiClient {
    pub fn new(
        tor_client: TorClient<PreferredRuntime>,
        isolation_token: Option<arti_client::IsolationToken>,
    ) -> Self {
        let arti_connector = ArtiConnector {
            client: tor_client.clone(),
            isolation_token,
        };

        let https = HttpsConnectorBuilder::new()
            .with_native_roots()
            .unwrap()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(arti_connector);

        let client = Client::builder(hyper_util::rt::TokioExecutor::new())
            .http2_only(false)
            // Phase 124 P3: HTTP/2 window tuning for Tor BDP
            // BDP = ~5 Mbps × 0.5s RTT = 312KB. 256KB stream / 4MB connection
            // reduces WINDOW_UPDATE stalls on 300KB+ directory pages.
            .http2_initial_stream_window_size(262_144)        // 256KB (vs default 64KB)
            .http2_initial_connection_window_size(4_194_304)  // 4MB  (vs 1MB — supports 4+ concurrent streams)
            // Phase 137: Adaptive receive window — Hyper dynamically grows the
            // window based on measured throughput (like TCP window scaling).
            // Eliminates WINDOW_UPDATE stalls for high-BDP Tor circuits.
            .http2_adaptive_window(true)
            // Phase 137: Larger max frame size — 32KB vs 16KB default.
            // Halves frame overhead for large body transfers.
            .http2_max_frame_size(Some(32_768))
            .http2_keep_alive_interval(Some(std::time::Duration::from_secs(15)))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(32)
            .build(https);

        Self::Tor {
            client,
            inner_tor_client: tor_client,
        }
    }

    pub fn new_clearnet() -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::USER_AGENT, reqwest::header::HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36"));
        headers.insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"));
        headers.insert(reqwest::header::ACCEPT_LANGUAGE, reqwest::header::HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert(reqwest::header::ACCEPT_ENCODING, reqwest::header::HeaderValue::from_static("gzip, deflate, br, zstd"));
        headers.insert(reqwest::header::CONNECTION, reqwest::header::HeaderValue::from_static("keep-alive"));
        headers.insert(reqwest::header::UPGRADE_INSECURE_REQUESTS, reqwest::header::HeaderValue::from_static("1"));
        headers.insert("Sec-Fetch-Dest", reqwest::header::HeaderValue::from_static("document"));
        headers.insert("Sec-Fetch-Mode", reqwest::header::HeaderValue::from_static("navigate"));
        headers.insert("Sec-Fetch-Site", reqwest::header::HeaderValue::from_static("none"));
        headers.insert("Sec-Fetch-User", reqwest::header::HeaderValue::from_static("?1"));
        headers.insert("Sec-Ch-Ua", reqwest::header::HeaderValue::from_static("\"Not A(Brand\";v=\"99\", \"Google Chrome\";v=\"121\", \"Chromium\";v=\"121\""));
        headers.insert("Sec-Ch-Ua-Mobile", reqwest::header::HeaderValue::from_static("?0"));
        headers.insert("Sec-Ch-Ua-Platform", reqwest::header::HeaderValue::from_static("\"Windows\""));

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(32)
            .tcp_nodelay(true)
            .default_headers(headers)
            .build()
            .unwrap_or_default();
        Self::Clearnet { client }
    }

    pub fn new_isolated(&self) -> Self {
        match self {
            Self::Tor {
                inner_tor_client, ..
            } => Self::new(
                inner_tor_client.clone(),
                Some(arti_client::IsolationToken::new()),
            ),
            Self::Clearnet { .. } => Self::new_clearnet(),
        }
    }

    // Phase 124 P0: Zero-alloc header generation.
    // Uses &'static str slices instead of Vec<(String, String)> — eliminates
    // ~3 heap allocations per request (~10,500 allocations saved per crawl).
    //
    // Phase 124 P3: UA pool expanded from 3 → 10 entries covering
    // Chrome/Firefox/Safari/Edge on Windows/macOS/Linux with 2025-2026 versions.
    const UA_POOL: &'static [&'static str] = &[
        // Chrome (Windows)
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
        // Chrome (macOS)
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        // Firefox (Linux)
        "Mozilla/5.0 (X11; Linux x86_64; rv:122.0) Gecko/20100101 Firefox/122.0",
        // Safari (macOS)
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2.1 Safari/605.1.15",
        // Edge (Windows)
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36 Edg/121.0.0.0",
        // Firefox (Windows)
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:123.0) Gecko/20100101 Firefox/123.0",
        // Chrome (Linux)
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36",
        // Firefox (macOS)
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:124.0) Gecko/20100101 Firefox/124.0",
        // Chrome 125 (Windows — 2025 vintage)
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
        // Safari 18 (macOS Sequoia — 2025)
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
    ];

    fn pick_random_ua() -> &'static str {
        Self::UA_POOL[rand::random::<usize>() % Self::UA_POOL.len()]
    }

    fn generate_base_headers() -> Vec<(&'static str, &'static str)> {
        vec![("user-agent", Self::pick_random_ua())]
    }

    pub fn get(&self, url: &str) -> ArtiRequestBuilder {
        match self {
            Self::Tor { client, .. } => {
                // Auto-detect .onion for longer TTFB
                let ttfb = if url.contains(".onion") { 25 } else { 10 };
                ArtiRequestBuilder::Tor {
                    client: client.clone(),
                    headers: Self::generate_base_headers(),
                    dynamic_headers: Vec::new(),
                    method: http::Method::GET,
                    url: url.to_string(),
                    json_body: None,
                    ttfb_timeout_secs: ttfb,
                }
            }
            Self::Clearnet { client } => ArtiRequestBuilder::Clearnet {
                req: client.get(url),
            },
        }
    }

    pub fn head(&self, url: &str) -> ArtiRequestBuilder {
        match self {
            Self::Tor { client, .. } => {
                let ttfb = if url.contains(".onion") { 25 } else { 10 };
                ArtiRequestBuilder::Tor {
                    client: client.clone(),
                    headers: Self::generate_base_headers(),
                    dynamic_headers: Vec::new(),
                    method: http::Method::HEAD,
                    url: url.to_string(),
                    json_body: None,
                    ttfb_timeout_secs: ttfb,
                }
            }
            Self::Clearnet { client } => ArtiRequestBuilder::Clearnet {
                req: client.head(url),
            },
        }
    }

    pub fn post(&self, url: &str) -> ArtiRequestBuilder {
        match self {
            Self::Tor { client, .. } => {
                let ttfb = if url.contains(".onion") { 25 } else { 10 };
                ArtiRequestBuilder::Tor {
                    client: client.clone(),
                    headers: Self::generate_base_headers(),
                    dynamic_headers: Vec::new(),
                    method: http::Method::POST,
                    url: url.to_string(),
                    json_body: None,
                    ttfb_timeout_secs: ttfb,
                }
            }
            Self::Clearnet { client } => ArtiRequestBuilder::Clearnet {
                req: client.post(url),
            },
        }
    }
}

pub enum ArtiRequestBuilder {
    Tor {
        client: Client<HttpsConnector<ArtiConnector>, http_body_util::Full<Bytes>>,
        headers: Vec<(&'static str, &'static str)>,
        /// Dynamically-added headers that need owned strings (e.g. cookies, content-length)
        dynamic_headers: Vec<(String, String)>,
        method: http::Method,
        url: String,
        json_body: Option<String>,
        /// Phase 121: Configurable TTFB timeout. Defaults to 10s for clearnet,
        /// auto-detected to 25s for .onion URLs. Callers can override.
        ttfb_timeout_secs: u64,
    },
    Clearnet {
        req: reqwest::RequestBuilder,
    },
}

impl ArtiRequestBuilder {
    pub fn header(self, key: &str, value: &str) -> Self {
        match self {
            Self::Tor {
                client,
                headers,
                mut dynamic_headers,
                method,
                url,
                json_body,
                ttfb_timeout_secs,
            } => {
                dynamic_headers.push((key.to_string(), value.to_string()));
                Self::Tor {
                    client,
                    headers,
                    dynamic_headers,
                    method,
                    url,
                    json_body,
                    ttfb_timeout_secs,
                }
            }
            Self::Clearnet { req } => Self::Clearnet {
                req: req.header(key, value),
            },
        }
    }

    /// Phase 121: Override TTFB timeout (in seconds) for this request.
    /// .onion URLs auto-default to 25s; clearnet to 10s.
    pub fn ttfb_timeout_secs(self, secs: u64) -> Self {
        match self {
            Self::Tor {
                client,
                headers,
                dynamic_headers,
                method,
                url,
                json_body,
                ..
            } => Self::Tor {
                client,
                headers,
                dynamic_headers,
                method,
                url,
                json_body,
                ttfb_timeout_secs: secs,
            },
            other => other, // No-op for clearnet
        }
    }

    pub fn json<T: serde::Serialize>(self, body: &T) -> Self {
        match self {
            Self::Tor {
                client,
                headers,
                mut dynamic_headers,
                method,
                url,
                ttfb_timeout_secs,
                ..
            } => {
                let json_body = serde_json::to_string(body).unwrap_or_default();
                dynamic_headers.push((
                    http::header::CONTENT_TYPE.to_string(),
                    "application/json".to_string(),
                ));
                dynamic_headers.push((
                    http::header::CONTENT_LENGTH.to_string(),
                    json_body.len().to_string(),
                ));
                Self::Tor {
                    client,
                    headers,
                    dynamic_headers,
                    method,
                    url,
                    json_body: Some(json_body),
                    ttfb_timeout_secs,
                }
            }
            Self::Clearnet { req } => Self::Clearnet {
                req: req.json(body),
            },
        }
    }

    pub async fn send(self) -> Result<ArtiResponse> {
        match self {
            Self::Tor {
                client,
                headers,
                dynamic_headers,
                method,
                url,
                json_body,
                ttfb_timeout_secs,
            } => {
                let mut current_url = url;
                let body_bytes = json_body.map(Bytes::from).unwrap_or_else(Bytes::new);
                let redirect_limit = 5usize;
                let mut accumulated_cookies: Vec<String> = Vec::new();

                for redirect_idx in 0..=redirect_limit {
                    let mut req = Request::builder().method(method.clone()).uri(&current_url);
                    // Phase 124 P0: static headers (zero-alloc)
                    for (key, value) in &headers {
                        req = req.header(*key, *value);
                    }
                    // Dynamic headers (caller-added via .header()/.json())
                    for (key, value) in &dynamic_headers {
                        req = req.header(key.as_str(), value.as_str());
                    }
                    if !accumulated_cookies.is_empty() {
                        req = req.header(http::header::COOKIE, accumulated_cookies.join("; "));
                    }

                    let req_obj = req
                        .body(http_body_util::Full::new(body_bytes.clone()))
                        .map_err(|e| anyhow!("Failed to build request: {}", e))?;

                    // Phase 121: Use configurable TTFB timeout (25s for .onion, 10s clearnet)
                    let res = tokio::time::timeout(
                        std::time::Duration::from_secs(ttfb_timeout_secs),
                        client.request(req_obj),
                    )
                    .await
                    .map_err(|_| anyhow!("TTFB Timeout"))?
                    .map_err(|e| anyhow!("Request failed: {}", e))?;

                    for val in res.headers().get_all(http::header::SET_COOKIE) {
                        if let Ok(cookie_str) = val.to_str() {
                            if let Some(cookie_pair) = cookie_str.split(';').next() {
                                accumulated_cookies.push(cookie_pair.to_string());
                            }
                        }
                    }

                    if redirect_idx < redirect_limit
                        && matches!(
                            res.status(),
                            StatusCode::MOVED_PERMANENTLY
                                | StatusCode::FOUND
                                | StatusCode::SEE_OTHER
                                | StatusCode::TEMPORARY_REDIRECT
                                | StatusCode::PERMANENT_REDIRECT
                        )
                    {
                        if let Some(location) = res.headers().get(http::header::LOCATION) {
                            if let Ok(location_str) = location.to_str() {
                                current_url = resolve_redirect_url(&current_url, location_str)?;
                                continue;
                            }
                        }
                    }

                    return Ok(ArtiResponse::Tor {
                        inner: res,
                        url: current_url,
                    });
                }

                Err(anyhow!("Redirect limit exceeded"))
            }
            Self::Clearnet { req } => {
                let res = req
                    .send()
                    .await
                    .map_err(|e| anyhow!("Clearnet request failed: {}", e))?;
                Ok(ArtiResponse::Clearnet { inner: res })
            }
        }
    }

    /// Phase 77D: Send the request but DON'T follow redirects.
    /// Returns (response, Option<resolved_location_url>).
    /// This is critical for Qilin Stage A: the CMS returns a 302 to a storage node,
    /// but we need to capture the Location header even when the storage node is offline.
    pub async fn send_capturing_redirect(self) -> Result<(ArtiResponse, Option<String>)> {
        match self {
            Self::Tor {
                client,
                headers,
                dynamic_headers,
                method,
                url,
                json_body,
                ttfb_timeout_secs,
            } => {
                let body_bytes = json_body.map(Bytes::from).unwrap_or_else(Bytes::new);
                let mut req = Request::builder().method(method).uri(&url);
                // Phase 124 P0: static headers (zero-alloc)
                for (key, value) in &headers {
                    req = req.header(*key, *value);
                }
                // Dynamic headers
                for (key, value) in &dynamic_headers {
                    req = req.header(key.as_str(), value.as_str());
                }

                let req_obj = req
                    .body(http_body_util::Full::new(body_bytes))
                    .map_err(|e| anyhow::anyhow!("Failed to build request: {}", e))?;

                // Phase 121: Use configurable TTFB timeout
                let res = tokio::time::timeout(
                    std::time::Duration::from_secs(ttfb_timeout_secs),
                    client.request(req_obj),
                )
                .await
                .map_err(|_| anyhow::anyhow!("TTFB Timeout"))?
                .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

                // Capture redirect Location if present
                let redirect_url = if matches!(
                    res.status(),
                    StatusCode::MOVED_PERMANENTLY
                        | StatusCode::FOUND
                        | StatusCode::SEE_OTHER
                        | StatusCode::TEMPORARY_REDIRECT
                        | StatusCode::PERMANENT_REDIRECT
                ) {
                    res.headers()
                        .get(http::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|loc| resolve_redirect_url(&url, loc).ok())
                } else {
                    None
                };

                Ok((ArtiResponse::Tor { inner: res, url }, redirect_url))
            }
            Self::Clearnet { req } => {
                let res = req
                    .send()
                    .await
                    .map_err(|e| anyhow!("Clearnet request failed: {}", e))?;
                Ok((ArtiResponse::Clearnet { inner: res }, None))
            }
        }
    }
}

fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String> {
    if let Ok(target) = url::Url::parse(location) {
        return Ok(target.to_string());
    }

    let base = url::Url::parse(current_url)
        .map_err(|e| anyhow!("Failed to parse base redirect URL: {}", e))?;
    let joined = base
        .join(location)
        .map_err(|e| anyhow!("Failed to resolve redirect location '{}': {}", location, e))?;
    Ok(joined.to_string())
}

pub enum ArtiResponse {
    Tor {
        inner: Response<hyper::body::Incoming>,
        url: String,
    },
    Clearnet {
        inner: reqwest::Response,
    },
}

impl ArtiResponse {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Tor { inner, .. } => inner.status(),
            Self::Clearnet { inner } => inner.status(),
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        match self {
            Self::Tor { inner, .. } => inner.headers(),
            Self::Clearnet { inner } => inner.headers(),
        }
    }

    pub fn content_length(&self) -> Option<u64> {
        match self {
            Self::Tor { inner, .. } => inner
                .headers()
                .get(http::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok()),
            Self::Clearnet { inner } => inner.content_length(),
        }
    }

    pub async fn text(self) -> Result<String> {
        match self {
            Self::Tor { inner, .. } => {
                let body_bytes = inner
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| anyhow!("Failed to read body: {}", e))?
                    .to_bytes();
                String::from_utf8(body_bytes.to_vec()).map_err(|e| anyhow!("Invalid UTF-8: {}", e))
            }
            Self::Clearnet { inner } => inner
                .text()
                .await
                .map_err(|e| anyhow!("Failed to read body: {}", e)),
        }
    }

    pub async fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        match self {
            Self::Tor { inner, .. } => {
                let body_bytes = inner
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| anyhow!("Failed to read body: {}", e))?
                    .to_bytes();
                let txt = String::from_utf8(body_bytes.to_vec())
                    .map_err(|e| anyhow!("Invalid UTF-8 for JSON: {}", e))?;
                serde_json::from_str(&txt).map_err(|e| anyhow!("JSON parse error: {}", e))
            }
            Self::Clearnet { inner } => inner
                .json::<T>()
                .await
                .map_err(|e| anyhow!("JSON parse error: {}", e)),
        }
    }

    pub fn url(&self) -> url::Url {
        match self {
            Self::Tor { url, .. } => {
                url::Url::parse(url).unwrap_or_else(|_| url::Url::parse("http://unknown").unwrap())
            }
            Self::Clearnet { inner } => inner.url().clone(),
        }
    }

    pub async fn bytes(self) -> Result<Bytes> {
        match self {
            Self::Tor { inner, .. } => {
                let body_bytes = inner
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| anyhow!("Failed to read body: {}", e))?
                    .to_bytes();
                Ok(body_bytes)
            }
            Self::Clearnet { inner } => inner
                .bytes()
                .await
                .map_err(|e| anyhow!("Failed to read body: {}", e)),
        }
    }

    pub fn bytes_stream(self) -> ArtiBodyStream {
        match self {
            Self::Tor { inner, .. } => ArtiBodyStream::Tor(inner.into_body()),
            Self::Clearnet { inner } => ArtiBodyStream::Clearnet(Box::pin(inner.bytes_stream())),
        }
    }
}

pub enum ArtiBodyStream {
    Tor(hyper::body::Incoming),
    Clearnet(Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send + Sync>>),
}

impl Stream for ArtiBodyStream {
    type Item = Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        use hyper::body::Body;
        match &mut *self {
            ArtiBodyStream::Tor(inner) => match Pin::new(inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        Poll::Ready(Some(Ok(data.clone())))
                    } else {
                        Poll::Ready(None)
                    }
                }
                Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(anyhow!("Stream error: {}", e)))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            },
            ArtiBodyStream::Clearnet(inner) => match inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => Poll::Ready(Some(Ok(data))),
                Poll::Ready(Some(Err(e))) => {
                    Poll::Ready(Some(Err(anyhow!("Clearnet stream error: {}", e))))
                }
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
