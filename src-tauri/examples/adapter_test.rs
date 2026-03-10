/// =============================================================================
/// CRAWLI ADAPTER CLI TEST HARNESS v1.0
/// =============================================================================
///
/// Comprehensive CLI test harness for individually verifying every directory listing
/// adapter against live high-latency decentralized storage endpoints.
///
/// Usage:
///   # Test a specific adapter with its canonical URL
///   cargo run --example adapter_test -- --adapter qilin
///
///   # Override the URL
///   cargo run --example adapter_test -- --adapter qilin --url "http://..."
///
///   # Test ALL adapters sequentially
///   cargo run --example adapter_test -- --all
///
///   # With options
///   cargo run --example adapter_test -- --adapter lockbit --circuits 24 --timeout-seconds 120
///
///   # JSON output
///   cargo run --example adapter_test -- --all --json
///
use crawli_lib::adapters::{AdapterRegistry, EntryType, FileEntry, SiteFingerprint};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::telemetry_bridge;
use crawli_lib::{tor, AppState};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;

// ─── Configuration ───────────────────────────────────────────────────────────

const DEFAULT_TOR_DAEMONS: usize = 4;
const DEFAULT_CIRCUITS: usize = 12;
const DEFAULT_TIMEOUT_SECS: u64 = 180;
const FINGERPRINT_MAX_RETRIES: usize = 3;
const HEALTH_PROBE_TIMEOUT_SECS: u64 = 45;
const ONION_TOURNAMENT_WAVE_SIZE: usize = 4;
const ONION_TOURNAMENT_MAX_CANDIDATES: usize = 12;
const CLEARNET_TOURNAMENT_WAVE_SIZE: usize = 12;
const CLEARNET_TOURNAMENT_MAX_CANDIDATES: usize = 60;

// ─── Canonical test URLs from adapter documentation ──────────────────────────

fn canonical_test_urls() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("qilin",       "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/");
    m.insert("lockbit",     "http://lockbit24pegjquuwbmwjlvyivmyaujf33kvlepcxyncnugm3zw73myd.onion/secret/123b67de858b6adc5dfdcfb2f6c4e8f7-caaf85ce-6aa7-370d-ba0c-25944d2230e3/manuaco.pt/unpack/");
    m.insert(
        "dragonforce",
        "http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/www.rjzavoral.com",
    );
    m.insert("worldleaks",  "https://worldleaksartrjm3c6vasllvgacbi5u3mgzkluehrzhk2jz4taufuid.onion/companies/9255855374/storage");
    m.insert(
        "abyss",
        "http://vmmefm7ktazj2bwtmy46o3wxhk42tctasyyqv6ymuzlivszteyhkkyad.onion/iamdesign.rar",
    );
    m.insert("alphalocker", "http://3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad.onion/gazomet.pl%20&%20cgas.pl/Files/");
    m.insert("inc_ransom",  "http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/698d5c538f1d14b7436dd63b");
    m.insert(
        "pear",
        "http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/",
    );
    m.insert(
        "play",
        "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp",
    );
    m
}

fn all_adapter_ids() -> Vec<&'static str> {
    vec![
        "qilin",
        "lockbit",
        "dragonforce",
        "worldleaks",
        "abyss",
        "alphalocker",
        "inc_ransom",
        "pear",
        "play",
    ]
}

fn is_onion_url(url: &str) -> bool {
    url.contains(".onion")
}

fn tournament_candidate_limit(url: &str, circuits: usize) -> usize {
    let dynamic = tor::tournament_candidate_count(circuits.max(1));
    if is_onion_url(url) {
        dynamic.clamp(
            circuits.max(ONION_TOURNAMENT_WAVE_SIZE),
            ONION_TOURNAMENT_MAX_CANDIDATES,
        )
    } else {
        dynamic.clamp(
            circuits.max(CLEARNET_TOURNAMENT_WAVE_SIZE),
            CLEARNET_TOURNAMENT_MAX_CANDIDATES,
        )
    }
}

fn tournament_wave_size(url: &str) -> usize {
    if is_onion_url(url) {
        ONION_TOURNAMENT_WAVE_SIZE
    } else {
        CLEARNET_TOURNAMENT_WAVE_SIZE
    }
}

// ─── Failure Classification ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum FailureClass {
    EndpointUnreachable(String),
    RateLimited(String),
    ParserEmpty(String),
    Timeout(String),
    RedirectLoop(String),
    Other(String),
}

impl std::fmt::Display for FailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailureClass::EndpointUnreachable(s) => write!(f, "ENDPOINT_UNREACHABLE: {}", s),
            FailureClass::RateLimited(s) => write!(f, "RATE_LIMITED: {}", s),
            FailureClass::ParserEmpty(s) => write!(f, "PARSER_EMPTY: {}", s),
            FailureClass::Timeout(s) => write!(f, "TIMEOUT: {}", s),
            FailureClass::RedirectLoop(s) => write!(f, "REDIRECT_LOOP: {}", s),
            FailureClass::Other(s) => write!(f, "OTHER: {}", s),
        }
    }
}

impl FailureClass {
    fn suggested_action(&self) -> &'static str {
        match self {
            FailureClass::EndpointUnreachable(_) => {
                "Endpoint appears unreachable — verify in Tor Browser, then retry later"
            }
            FailureClass::RateLimited(_) => {
                "Endpoint throttling/blocking — reduce circuits or try later"
            }
            FailureClass::ParserEmpty(_) => {
                "Adapter matched but parsed 0 entries — possible adapter regression or site change"
            }
            FailureClass::Timeout(_) => {
                "Crawl exceeded time limit — increase --timeout-seconds or check Tor health"
            }
            FailureClass::RedirectLoop(_) => {
                "Endpoint returned repeated redirects — check URL or site status"
            }
            FailureClass::Other(_) => "Unexpected error — inspect diagnostic logs for root cause",
        }
    }

    fn category_tag(&self) -> &'static str {
        match self {
            FailureClass::EndpointUnreachable(_) => "NETWORK",
            FailureClass::RateLimited(_) => "SERVER",
            FailureClass::ParserEmpty(_) => "ADAPTER",
            FailureClass::Timeout(_) => "TIMEOUT",
            FailureClass::RedirectLoop(_) => "NETWORK",
            FailureClass::Other(_) => "UNKNOWN",
        }
    }
}

fn classify_error(error_msg: &str) -> FailureClass {
    let lower = error_msg.to_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        return FailureClass::EndpointUnreachable(error_msg.to_string());
    }
    if lower.contains("connection refused")
        || lower.contains("reset")
        || lower.contains("broken pipe")
    {
        return FailureClass::EndpointUnreachable(error_msg.to_string());
    }
    if lower.contains("hidden service") || lower.contains("descriptor") || lower.contains("circuit")
    {
        return FailureClass::EndpointUnreachable(error_msg.to_string());
    }
    if lower.contains("connect") && lower.contains("error") {
        return FailureClass::EndpointUnreachable(error_msg.to_string());
    }
    if lower.contains("429") || lower.contains("503") || lower.contains("403") {
        return FailureClass::RateLimited(error_msg.to_string());
    }
    if lower.contains("redirect") {
        return FailureClass::RedirectLoop(error_msg.to_string());
    }
    FailureClass::Other(error_msg.to_string())
}

// ─── Test Results ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AdapterTestResult {
    adapter_id: String,
    adapter_name: String,
    url: String,
    status: TestStatus,
    matched_adapter: Option<String>,
    total_entries: usize,
    total_files: usize,
    total_folders: usize,
    total_size_bytes: u64,
    max_depth: usize,
    elapsed_secs: f64,
    entries_per_second: f64,
    fingerprint_status: Option<u16>,
    fingerprint_body_len: Option<usize>,
    fingerprint_elapsed_secs: f64,
    successful_requests: usize,
    failed_requests: usize,
    failure_class: Option<FailureClass>,
    diagnostic_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum TestStatus {
    Success,
    Partial,
    Failed,
}

impl std::fmt::Display for TestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestStatus::Success => write!(f, "SUCCESS"),
            TestStatus::Partial => write!(f, "PARTIAL"),
            TestStatus::Failed => write!(f, "FAILED"),
        }
    }
}

impl AdapterTestResult {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "adapter_id": self.adapter_id,
            "adapter_name": self.adapter_name,
            "url": self.url,
            "status": format!("{}", self.status),
            "matched_adapter": self.matched_adapter,
            "total_entries": self.total_entries,
            "total_files": self.total_files,
            "total_folders": self.total_folders,
            "total_size_bytes": self.total_size_bytes,
            "max_depth": self.max_depth,
            "elapsed_secs": self.elapsed_secs,
            "entries_per_second": self.entries_per_second,
            "fingerprint_status": self.fingerprint_status,
            "fingerprint_body_len": self.fingerprint_body_len,
            "fingerprint_elapsed_secs": self.fingerprint_elapsed_secs,
            "successful_requests": self.successful_requests,
            "failed_requests": self.failed_requests,
            "failure_class": self.failure_class.as_ref().map(|fc| format!("{}", fc)),
            "failure_category": self.failure_class.as_ref().map(|fc| fc.category_tag()),
            "suggested_action": self.failure_class.as_ref().map(|fc| fc.suggested_action()),
            "diagnostic_notes": self.diagnostic_notes,
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn compute_max_depth(entries: &[FileEntry]) -> usize {
    entries
        .iter()
        .map(|e| e.path.matches('/').count())
        .max()
        .unwrap_or(0)
}

// ─── CLI Argument Parsing ────────────────────────────────────────────────────

struct CliArgs {
    adapter: Option<String>,
    url: Option<String>,
    all: bool,
    circuits: usize,
    timeout_seconds: u64,
    json_output: bool,
    daemons: usize,
}

fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    let mut cli = CliArgs {
        adapter: None,
        url: None,
        all: false,
        circuits: DEFAULT_CIRCUITS,
        timeout_seconds: DEFAULT_TIMEOUT_SECS,
        json_output: false,
        daemons: DEFAULT_TOR_DAEMONS,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--adapter" | "-a" => {
                i += 1;
                cli.adapter = args.get(i).cloned();
            }
            "--url" | "-u" => {
                i += 1;
                cli.url = args.get(i).cloned();
            }
            "--all" => cli.all = true,
            "--circuits" | "-c" => {
                i += 1;
                cli.circuits = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(DEFAULT_CIRCUITS);
            }
            "--timeout-seconds" | "-t" => {
                i += 1;
                cli.timeout_seconds = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(DEFAULT_TIMEOUT_SECS);
            }
            "--daemons" | "-d" => {
                i += 1;
                cli.daemons = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(DEFAULT_TOR_DAEMONS);
            }
            "--json" | "-j" => cli.json_output = true,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    if !cli.all && cli.adapter.is_none() {
        eprintln!("ERROR: Specify --adapter <name> or --all\n");
        print_usage();
        std::process::exit(1);
    }

    cli
}

fn print_usage() {
    eprintln!("=== CRAWLI ADAPTER CLI TEST HARNESS v1.0 ===\n");
    eprintln!("USAGE:");
    eprintln!("  cargo run --example adapter_test -- [OPTIONS]\n");
    eprintln!("OPTIONS:");
    eprintln!("  --adapter, -a <ID>       Test a specific adapter (e.g. qilin, lockbit)");
    eprintln!("  --url, -u <URL>          Override the canonical test URL");
    eprintln!("  --all                    Test ALL adapters sequentially");
    eprintln!(
        "  --circuits, -c <N>       Tor circuits (default: {})",
        DEFAULT_CIRCUITS
    );
    eprintln!(
        "  --timeout-seconds, -t    Max seconds per adapter (default: {})",
        DEFAULT_TIMEOUT_SECS
    );
    eprintln!(
        "  --daemons, -d <N>        Tor daemons (default: {})",
        DEFAULT_TOR_DAEMONS
    );
    eprintln!("  --json, -j               Output results as JSON");
    eprintln!("  --help, -h               Show this help\n");
    eprintln!("AVAILABLE ADAPTERS:");
    let canonical = canonical_test_urls();
    for id in all_adapter_ids() {
        let url = canonical.get(id).unwrap_or(&"<no canonical URL>");
        eprintln!("  {:16} {}", id, url);
    }
    eprintln!();
}

// ─── Core Test Runner ────────────────────────────────────────────────────────

async fn run_adapter_test(
    adapter_id: &str,
    test_url: &str,
    app_handle: &tauri::AppHandle,
    arti_clients: &[crawli_lib::tor_native::SharedTorClient],
    active_ports: &[u16],
    circuits: usize,
    daemons: usize,
    timeout_seconds: u64,
) -> AdapterTestResult {
    let mut result = AdapterTestResult {
        adapter_id: adapter_id.to_string(),
        adapter_name: String::new(),
        url: test_url.to_string(),
        status: TestStatus::Failed,
        matched_adapter: None,
        total_entries: 0,
        total_files: 0,
        total_folders: 0,
        total_size_bytes: 0,
        max_depth: 0,
        elapsed_secs: 0.0,
        entries_per_second: 0.0,
        fingerprint_status: None,
        fingerprint_body_len: None,
        fingerprint_elapsed_secs: 0.0,
        successful_requests: 0,
        failed_requests: 0,
        failure_class: None,
        diagnostic_notes: Vec::new(),
    };

    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(circuits),
        daemons: Some(daemons),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: true,
    };

    let daemon_count = active_ports.len().max(arti_clients.len()).max(1);

    let frontier = CrawlerFrontier::new(
        Some(app_handle.clone()),
        test_url.to_string(),
        daemon_count,
        true, // all our test URLs are onion
        active_ports.to_vec(),
        arti_clients.to_vec(),
        options,
        None,
    );

    let test_start = Instant::now();

    // ─── Phase 1: High-Frequency Tournament Selection ────────────────────
    let tournament_candidates = tournament_candidate_limit(test_url, circuits);
    let tournament_wave = tournament_wave_size(test_url).min(tournament_candidates.max(1));
    let target_winners = 3.min(tournament_candidates.max(1));
    let mut recorded_latencies_ms = Vec::new();

    log_phase(
        adapter_id,
        "TOURNAMENT",
        &format!(
            "Spinning {} isolated circuits in waves of {} to find the fastest Tor path...",
            tournament_candidates, tournament_wave
        ),
    );
    let _tournament_start = Instant::now();
    let mut winners = Vec::new();
    let mut next_candidate = 0usize;
    let total_waves = tournament_candidates.div_ceil(tournament_wave);

    'wave_probe: while next_candidate < tournament_candidates && winners.len() < target_winners {
        let wave_start = next_candidate;
        let wave_end = (wave_start + tournament_wave).min(tournament_candidates);
        log_phase(
            adapter_id,
            "TOURNAMENT",
            &format!(
                "Wave {}/{}: probing candidates {}-{}",
                (wave_start / tournament_wave) + 1,
                total_waves,
                wave_start + 1,
                wave_end
            ),
        );

        let mut race_tasks = tokio::task::JoinSet::new();
        let mut accumulated_jitter_ms: u64 = 0;

        for i in wave_start..wave_end {
            let (cid, client) = frontier.get_client();
            let target_url_clone = test_url.to_string();

            use rand::Rng;

            let my_jitter = if i == wave_start || i == wave_start + 1 {
                0
            } else if i % 2 == 1 {
                accumulated_jitter_ms
            } else {
                let step = rand::thread_rng().gen_range(50..=150) as u64;
                accumulated_jitter_ms + step
            };
            accumulated_jitter_ms = my_jitter;

            race_tasks.spawn(async move {
                if my_jitter > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(my_jitter)).await;
                }

                let start = tokio::time::Instant::now();
                let res = tokio::time::timeout(
                    Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS),
                    // Use GET over HEAD since many misconfigured nginx Tor sites drop HTTP HEAD payloads
                    client.get(&target_url_clone).send(),
                )
                .await;
                (i, cid, client, start.elapsed().as_millis(), res)
            });
        }

        while let Some(res) = race_tasks.join_next().await {
            if let Ok((_i, cid, client, latency, Ok(Ok(resp)))) = res {
                winners.push((cid, client, latency, resp.status().as_u16()));
                recorded_latencies_ms.push(latency as u64);
                log_phase(
                    adapter_id,
                    "TOURNAMENT",
                    &format!("✓ Circuit C-{} won! Latency: {}ms", cid, latency),
                );
                if winners.len() >= target_winners {
                    race_tasks.abort_all();
                    break 'wave_probe;
                }
            }
        }

        race_tasks.abort_all();
        next_candidate = wave_end;
    }

    tor::update_tournament_telemetry(&recorded_latencies_ms, winners.len(), tournament_candidates);

    if winners.is_empty() {
        log_phase(
            adapter_id,
            "TOURNAMENT",
            &format!(
                "No winning circuit in {} candidates. Falling back to {} sequential rotated probes...",
                tournament_candidates, FINGERPRINT_MAX_RETRIES
            ),
        );

        for attempt in 1..=FINGERPRINT_MAX_RETRIES {
            let (cid, client) = frontier.get_client();
            let start = tokio::time::Instant::now();
            match tokio::time::timeout(
                Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS),
                client.get(test_url).send(),
            )
            .await
            {
                Ok(Ok(resp)) => {
                    let latency = start.elapsed().as_millis();
                    winners.push((cid, client, latency, resp.status().as_u16()));
                    frontier.record_success(cid, 0, latency as u64);
                    log_phase(
                        adapter_id,
                        "TOURNAMENT",
                        &format!(
                            "✓ Sequential fallback winner on attempt {}/{} via C-{} ({}ms)",
                            attempt, FINGERPRINT_MAX_RETRIES, cid, latency
                        ),
                    );
                    break;
                }
                Ok(Err(err)) => {
                    log_phase(
                        adapter_id,
                        "TOURNAMENT",
                        &format!(
                            "Sequential fallback attempt {}/{} failed: {}",
                            attempt, FINGERPRINT_MAX_RETRIES, err
                        ),
                    );
                }
                Err(_) => {
                    log_phase(
                        adapter_id,
                        "TOURNAMENT",
                        &format!(
                            "Sequential fallback attempt {}/{} timed out after {}s",
                            attempt, FINGERPRINT_MAX_RETRIES, HEALTH_PROBE_TIMEOUT_SECS
                        ),
                    );
                }
            }

            if attempt < FINGERPRINT_MAX_RETRIES {
                tokio::time::sleep(Duration::from_millis((attempt as u64) * 750)).await;
            }
        }
    }

    if winners.is_empty() {
        let fc = FailureClass::EndpointUnreachable(format!(
            "All {} tournament circuits failed and {} sequential fallback probes failed.",
            tournament_candidates, FINGERPRINT_MAX_RETRIES
        ));
        log_phase(adapter_id, "TOURNAMENT", &format!("✗ FAILED: {}", fc));
        result.failure_class = Some(fc);
        return result;
    }

    // ─── Phase 2: Fingerprint Acquisition (Using Fastest Winner) ─────────
    let (fastest_cid, fastest_client, fastest_latency, _) = winners.remove(0);
    frontier.record_success(fastest_cid, 0, fastest_latency as u64);
    result.diagnostic_notes.push(format!(
        "Tournament Winner: Circuit {} in {}ms",
        fastest_cid, fastest_latency
    ));

    log_phase(
        adapter_id,
        "FP",
        &format!(
            "Acquiring site fingerprint via fastest circuit C-{}...",
            fastest_cid
        ),
    );
    let fp_start = Instant::now();

    let mut fingerprint_ok: Option<SiteFingerprint> = None;

    let req_result =
        tokio::time::timeout(Duration::from_secs(30), fastest_client.get(test_url).send()).await;

    match req_result {
        Ok(Ok(resp)) => {
            let status = resp.status().as_u16();
            let headers = resp.headers().clone();

            let content_type = headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            let server_header = headers
                .get("server")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            log_phase(
                adapter_id,
                "FP",
                &format!(
                    "HTTP {} | Content-Type: {} | Server: {}",
                    status, content_type, server_header
                ),
            );

            let is_binary = !content_type.starts_with("text/")
                && !content_type.contains("json")
                && !content_type.contains("xml")
                && (content_type.contains("application/")
                    || test_url.ends_with(".7z")
                    || test_url.ends_with(".zip")
                    || test_url.ends_with(".rar"));

            let body = if is_binary {
                "[BINARY_OR_ARCHIVE_DATA]".to_string()
            } else {
                match tokio::time::timeout(Duration::from_secs(30), resp.text()).await {
                    Ok(Ok(text)) => text,
                    Ok(Err(e)) => {
                        result.failed_requests += 1;
                        log_phase(adapter_id, "FP", &format!("Body read error: {}", e));
                        "".to_string()
                    }
                    Err(_) => {
                        result.failed_requests += 1;
                        log_phase(adapter_id, "FP", "Body read timeout");
                        "".to_string()
                    }
                }
            };

            if !body.is_empty() {
                frontier.record_success(
                    fastest_cid,
                    body.len() as u64,
                    fp_start.elapsed().as_millis() as u64,
                );
                result.fingerprint_status = Some(status);
                result.fingerprint_body_len = Some(body.len());
                result.successful_requests += 1;

                fingerprint_ok = Some(SiteFingerprint {
                    url: test_url.to_string(),
                    status,
                    headers,
                    body,
                });
            }
        }
        Ok(Err(e)) => {
            result.failed_requests += 1;
            frontier.record_failure(fastest_cid);
            log_phase(adapter_id, "FP", &format!("Request error: {}", e));
        }
        Err(_) => {
            result.failed_requests += 1;
            frontier.record_failure(fastest_cid);
            log_phase(adapter_id, "FP", "Request timeout (30s)");
        }
    }

    let fingerprint = match fingerprint_ok {
        Some(fp) => {
            result.fingerprint_elapsed_secs = fp_start.elapsed().as_secs_f64();
            log_phase(
                adapter_id,
                "FP",
                &format!(
                    "✓ Fingerprint acquired in {:.2}s (HTTP {}, {} bytes body)",
                    result.fingerprint_elapsed_secs,
                    fp.status,
                    fp.body.len()
                ),
            );
            fp
        }
        None => {
            result.fingerprint_elapsed_secs = fp_start.elapsed().as_secs_f64();
            let fc = FailureClass::EndpointUnreachable(format!(
                "All {} fingerprint attempts failed in {:.2}s",
                FINGERPRINT_MAX_RETRIES, result.fingerprint_elapsed_secs
            ));
            log_phase(adapter_id, "FP", &format!("✗ FAILED: {}", fc));
            result.diagnostic_notes.push(format!("Fingerprint: {}", fc));
            result
                .diagnostic_notes
                .push(format!("→ {}", fc.suggested_action()));
            result.failure_class = Some(fc);
            result.elapsed_secs = test_start.elapsed().as_secs_f64();
            return result;
        }
    };

    // ─── Phase 3: Adapter Detection ──────────────────────────────────────
    log_phase(
        adapter_id,
        "MATCH",
        "Running multi-tier adapter classification...",
    );
    let registry = AdapterRegistry::new();
    let adapter = match registry.determine_adapter(&fingerprint).await {
        Some(a) => {
            result.matched_adapter = Some(a.name().to_string());
            result.adapter_name = a.name().to_string();
            log_phase(adapter_id, "MATCH", &format!("✓ Matched: {}", a.name()));
            a
        }
        None => {
            let body_preview = &fingerprint.body[..fingerprint.body.len().min(300)];
            let fc = FailureClass::ParserEmpty("No adapter matched the fingerprint".to_string());
            log_phase(adapter_id, "MATCH", &format!("✗ {}", fc));
            result
                .diagnostic_notes
                .push(format!("Body preview: {}", body_preview));
            result
                .diagnostic_notes
                .push(format!("→ {}", fc.suggested_action()));
            result.failure_class = Some(fc);
            result.elapsed_secs = test_start.elapsed().as_secs_f64();
            return result;
        }
    };

    // ─── Phase 4: Live Crawl ─────────────────────────────────────────────
    log_phase(
        adapter_id,
        "CRAWL",
        &format!(
            "Starting live crawl ({}s limit, {} circuits)...",
            timeout_seconds, circuits
        ),
    );
    let crawl_start = Instant::now();
    let frontier_arc = Arc::new(frontier);

    let crawl_result = tokio::time::timeout(
        Duration::from_secs(timeout_seconds),
        adapter.crawl(test_url, frontier_arc.clone(), app_handle.clone()),
    )
    .await;

    let crawl_elapsed = crawl_start.elapsed().as_secs_f64();
    result.elapsed_secs = test_start.elapsed().as_secs_f64();
    result.successful_requests += frontier_arc.processed_count();

    match crawl_result {
        Ok(Ok(files)) => {
            result.total_files = files
                .iter()
                .filter(|e| matches!(e.entry_type, EntryType::File))
                .count();
            result.total_folders = files
                .iter()
                .filter(|e| matches!(e.entry_type, EntryType::Folder))
                .count();
            result.total_entries = files.len();
            result.total_size_bytes = files.iter().filter_map(|e| e.size_bytes).sum();
            result.max_depth = compute_max_depth(&files);
            result.entries_per_second = if crawl_elapsed > 0.0 {
                result.total_entries as f64 / crawl_elapsed
            } else {
                0.0
            };

            if result.total_entries > 0 {
                result.status = TestStatus::Success;
                log_phase(
                    adapter_id,
                    "CRAWL",
                    &format!(
                    "✓ SUCCESS: {} entries ({} files, {} folders) depth={} in {:.2}s — {:.2} ent/s",
                    result.total_entries, result.total_files, result.total_folders,
                    result.max_depth, crawl_elapsed, result.entries_per_second,
                ),
                );
                // Log first few entries as sample
                for (i, entry) in files.iter().take(5).enumerate() {
                    let type_tag = match entry.entry_type {
                        EntryType::File => "FILE",
                        EntryType::Folder => "DIR ",
                    };
                    let size_str = entry
                        .size_bytes
                        .map_or("?".to_string(), |b| format!("{}", b));
                    log_phase(
                        adapter_id,
                        "CRAWL",
                        &format!("  [{}] {} {} ({}B)", i + 1, type_tag, entry.path, size_str),
                    );
                }
                if files.len() > 5 {
                    log_phase(
                        adapter_id,
                        "CRAWL",
                        &format!("  ... and {} more entries", files.len() - 5),
                    );
                }
            } else {
                // ─── Zero-Entry Diagnosis ──────────────────────────
                log_phase(
                    adapter_id,
                    "DIAG",
                    "⚠ Crawl returned 0 entries — diagnosing...",
                );

                let fc = if result.fingerprint_status == Some(200) {
                    result.diagnostic_notes.push(format!(
                        "HTTP 200 body={} bytes but adapter produced 0 entries",
                        result.fingerprint_body_len.unwrap_or(0)
                    ));
                    result.diagnostic_notes.push(format!(
                        "Body preview: {}",
                        &fingerprint.body[..fingerprint.body.len().min(300)]
                    ));
                    FailureClass::ParserEmpty("HTTP 200 but adapter returned empty Vec".to_string())
                } else {
                    FailureClass::Other(format!(
                        "Crawl returned 0 entries with fingerprint HTTP {}",
                        result.fingerprint_status.unwrap_or(0)
                    ))
                };
                result
                    .diagnostic_notes
                    .push(format!("→ {}", fc.suggested_action()));
                result.failure_class = Some(fc);
            }
        }
        Ok(Err(e)) => {
            let error_msg = e.to_string();
            let fc = classify_error(&error_msg);
            log_phase(adapter_id, "CRAWL", &format!("✗ ERROR: {}", fc));
            result
                .diagnostic_notes
                .push(format!("Crawl error: {}", error_msg));
            result
                .diagnostic_notes
                .push(format!("→ {}", fc.suggested_action()));
            result.failure_class = Some(fc);
        }
        Err(_) => {
            // Timeout — check if partial progress was made
            let visited = frontier_arc.visited_count();
            let processed = frontier_arc.processed_count();
            if visited > 0 || processed > 0 {
                result.status = TestStatus::Partial;
                result.total_entries = visited;
                result.entries_per_second = if crawl_elapsed > 0.0 {
                    visited as f64 / crawl_elapsed
                } else {
                    0.0
                };
                let fc = FailureClass::Timeout(format!(
                    "Hit {}s limit — visited={} processed={}",
                    timeout_seconds, visited, processed
                ));
                log_phase(adapter_id, "CRAWL", &format!("◐ PARTIAL: {}", fc));
                result
                    .diagnostic_notes
                    .push("Increase --timeout-seconds to allow more discovery".to_string());
                result
                    .diagnostic_notes
                    .push(format!("→ {}", fc.suggested_action()));
                result.failure_class = Some(fc);
            } else {
                let fc = FailureClass::Timeout(format!(
                    "Crawl timed out after {}s with no results",
                    timeout_seconds
                ));
                log_phase(adapter_id, "CRAWL", &format!("✗ FAILED: {}", fc));
                result
                    .diagnostic_notes
                    .push(format!("→ {}", fc.suggested_action()));
                result.failure_class = Some(fc);
            }
        }
    }

    frontier_arc.cancel();
    result
}

// ─── Logging ─────────────────────────────────────────────────────────────────

fn log_phase(_adapter_id: &str, phase: &str, message: &str) {
    let ts = chrono::Local::now().format("%H:%M:%S%.3f");
    eprintln!("  [{} | {:>5}] {}", ts, phase, message);
}

fn print_divider(ch: char, width: usize) {
    println!("{}", std::iter::repeat_n(ch, width).collect::<String>());
}

// ─── Summary Table ───────────────────────────────────────────────────────────

fn print_summary_table(results: &[AdapterTestResult]) {
    println!();
    print_divider('═', 150);
    println!("  ADAPTER TEST RESULTS SUMMARY");
    print_divider('═', 150);

    println!(
        "  {:<16} {:<28} {:<9} {:>7} {:>7} {:>7} {:>5} {:>8} {:>8} {:<10} {:<40}",
        "ADAPTER",
        "MATCHED",
        "STATUS",
        "TOTAL",
        "FILES",
        "DIRS",
        "DEPTH",
        "TIME",
        "ENT/s",
        "CATEGORY",
        "NEXT STEP"
    );
    print_divider('─', 150);

    for r in results {
        let fail_tag = r
            .failure_class
            .as_ref()
            .map(|fc| fc.category_tag().to_string())
            .unwrap_or_else(|| "—".to_string());
        let next_step = r
            .failure_class
            .as_ref()
            .map(|fc| {
                let a = fc.suggested_action();
                if a.len() > 38 {
                    format!("{}...", &a[..35])
                } else {
                    a.to_string()
                }
            })
            .unwrap_or_else(|| "—".to_string());
        let name = r.matched_adapter.as_deref().unwrap_or("—");
        let name_disp = if name.len() > 26 {
            format!("{}...", &name[..23])
        } else {
            name.to_string()
        };

        println!(
            "  {:<16} {:<28} {:<9} {:>7} {:>7} {:>7} {:>5} {:>7.1}s {:>8.2} {:<10} {:<40}",
            r.adapter_id,
            name_disp,
            format!("{}", r.status),
            r.total_entries,
            r.total_files,
            r.total_folders,
            r.max_depth,
            r.elapsed_secs,
            r.entries_per_second,
            fail_tag,
            next_step,
        );
    }

    print_divider('─', 150);

    let total_success = results
        .iter()
        .filter(|r| r.status == TestStatus::Success)
        .count();
    let total_partial = results
        .iter()
        .filter(|r| r.status == TestStatus::Partial)
        .count();
    let total_failed = results
        .iter()
        .filter(|r| r.status == TestStatus::Failed)
        .count();
    let total_entries: usize = results.iter().map(|r| r.total_entries).sum();

    println!();
    println!("  AGGREGATE: {} tested | {} SUCCESS | {} PARTIAL | {} FAILED | {} total entries discovered",
        results.len(), total_success, total_partial, total_failed, total_entries);
    print_divider('═', 150);
}

fn print_detailed_diagnostics(results: &[AdapterTestResult]) {
    let has_diagnostics = results.iter().any(|r| !r.diagnostic_notes.is_empty());
    if !has_diagnostics {
        return;
    }

    println!();
    println!("  DETAILED DIAGNOSTICS");
    print_divider('─', 80);

    for r in results {
        if r.diagnostic_notes.is_empty() {
            continue;
        }
        println!();
        println!("  ┌── {} ({}) ──", r.adapter_id, r.status);
        for note in &r.diagnostic_notes {
            println!("  │  {}", note);
        }
        println!("  └──");
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let cli = parse_args();

    println!();
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║         CRAWLI ADAPTER CLI TEST HARNESS v1.0                        ║");
    println!("║         Directory listing adapter validation for archival research  ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!();

    // Build Tauri app on the main thread (macOS EventLoop requirement)
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .expect("Failed to build tauri app for test harness");
    let app_handle = app.handle().clone();
    let bridge = app.state::<AppState>().telemetry_bridge.clone();

    // Multi-threaded runtime (required by frontier's block_in_place)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        telemetry_bridge::spawn_bridge_emitter(app_handle.clone(), bridge);

        // Determine which adapters to test
        let canonical = canonical_test_urls();
        let adapters_to_test: Vec<(String, String)> = if cli.all {
            all_adapter_ids()
                .iter()
                .map(|id| {
                    let url = canonical.get(id).unwrap_or(&"").to_string();
                    (id.to_string(), url)
                })
                .collect()
        } else {
            let id = cli.adapter.as_deref().unwrap();
            let url = cli
                .url
                .clone()
                .unwrap_or_else(|| canonical.get(id).unwrap_or(&"").to_string());
            if url.is_empty() {
                eprintln!("ERROR: No canonical URL for '{}'. Use --url.", id);
                std::process::exit(1);
            }
            vec![(id.to_string(), url)]
        };

        println!(
            "  Adapters:  {}",
            adapters_to_test
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("  Circuits:  {}", cli.circuits);
        println!("  Timeout:   {}s per adapter", cli.timeout_seconds);
        println!("  Daemons:   {}", cli.daemons);
        println!();

        // ─── Bootstrap Tor ───────────────────────────────────────────────
        println!("[TOR] Cleaning stale daemons...");
        tor::cleanup_stale_tor_daemons();

        println!("[TOR] Bootstrapping {}-daemon arti swarm...", cli.daemons);
        let tor_start = Instant::now();
        let (guard, active_ports) =
            match tor::bootstrap_tor_cluster(app_handle.clone(), cli.daemons, 0).await {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("[TOR] FATAL: Bootstrap failed: {}", e);
                    eprintln!(
                        "[TOR] Check network connectivity and arti state directory permissions."
                    );
                    std::process::exit(1);
                }
            };
        let tor_elapsed = tor_start.elapsed().as_secs_f64();
        let arti_clients = guard.get_arti_clients();
        println!(
            "[TOR] ✓ Ready in {:.2}s — {} live client(s), {} port(s)\n",
            tor_elapsed,
            arti_clients.len(),
            active_ports.len()
        );

        let mut all_results: Vec<AdapterTestResult> = Vec::new();

        // ─── Run Tests ───────────────────────────────────────────────────
        for (idx, (adapter_id, url)) in adapters_to_test.iter().enumerate() {
            println!();
            print_divider('─', 100);
            println!(
                "  [{}/{}] Testing adapter: {}",
                idx + 1,
                adapters_to_test.len(),
                adapter_id
            );
            println!("  URL: {}", url);
            print_divider('─', 100);

            let result = run_adapter_test(
                adapter_id,
                url,
                &app_handle,
                &arti_clients,
                &active_ports,
                cli.circuits,
                cli.daemons,
                cli.timeout_seconds,
            )
            .await;

            // Print per-adapter summary line
            println!();
            let status_icon = match result.status {
                TestStatus::Success => "✓",
                TestStatus::Partial => "◐",
                TestStatus::Failed => "✗",
            };
            println!(
                "  {} {} | {} entries ({} files, {} folders) | depth {} | {:.2}s | {:.2} ent/s",
                status_icon,
                result.status,
                result.total_entries,
                result.total_files,
                result.total_folders,
                result.max_depth,
                result.elapsed_secs,
                result.entries_per_second,
            );
            if let Some(ref fc) = result.failure_class {
                println!("    [{}] {}", fc.category_tag(), fc);
            }

            all_results.push(result);
        }

        // ─── Output ─────────────────────────────────────────────────────
        if cli.json_output {
            let json_results: Vec<_> = all_results.iter().map(|r| r.to_json()).collect();
            let output = serde_json::json!({
                "harness_version": "1.0",
                "timestamp": chrono::Local::now().to_rfc3339(),
                "tor_bootstrap_secs": tor_elapsed,
                "circuits": cli.circuits,
                "timeout_seconds": cli.timeout_seconds,
                "daemons": cli.daemons,
                "results": json_results,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        } else {
            print_summary_table(&all_results);
            print_detailed_diagnostics(&all_results);
        }

        println!("\n=== END OF HARNESS ===\n");
    });
}
