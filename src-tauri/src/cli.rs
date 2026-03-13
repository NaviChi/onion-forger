#[cfg(not(test))]
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
#[cfg(not(test))]
use base64::Engine;
#[cfg(not(test))]
use clap::CommandFactory;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use crate::frontier::DownloadMode;
#[cfg(not(test))]
use serde_json::json;
#[cfg(not(test))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::{Arc, Mutex};
#[cfg(not(test))]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(not(test))]
use tauri::{Listener, Manager};

#[derive(Debug, Parser)]
#[command(
    name = "crawli",
    about = "Headless CLI entrypoint for the Crawli backend",
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Suppress streamed backend events on stderr"
    )]
    quiet_events: bool,
    #[arg(
        long,
        global = true,
        help = "Emit compact JSON instead of pretty JSON on stdout"
    )]
    compact_json: bool,
    #[arg(
        long,
        global = true,
        help = "Also stream telemetry_bridge_update frames on stderr"
    )]
    include_telemetry_events: bool,
    #[arg(
        long,
        global = true,
        help = "Emit condensed live progress summaries on stderr"
    )]
    progress_summary: bool,
    #[arg(
        long,
        global = true,
        default_value_t = 2000,
        help = "Minimum interval between compact progress summaries in milliseconds"
    )]
    progress_summary_interval_ms: u64,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Crawl(CrawlArgs),
    DownloadFiles(DownloadFilesArgs),
    DownloadAll(DownloadAllArgs),
    InitiateDownload(InitiateDownloadArgs),
    CancelCrawl,
    PauseDownload,
    StopDownload,
    ExportJson(ExportJsonArgs),
    GetVfsChildren(GetVfsChildrenArgs),
    AdapterCatalog,
    IngestVfsEntries(IngestVfsEntriesArgs),
    PreResolve(PreResolveArgs),
    SubtreeHeatmap(SubtreeHeatmapArgs),
    OpenFolder(OpenFolderArgs),
    SetTelemetryEnabled(SetTelemetryEnabledArgs),
    DrainTelemetryRing(DrainTelemetryRingArgs),
    DetectInputMode(DetectInputModeArgs),
    SystemProfile,
    FetchNetworkDiskBlock(FetchNetworkDiskBlockArgs),
    FetchNetworkDiskExtents(FetchNetworkDiskExtentsArgs),
    #[cfg(feature = "azure")]
    ConfigureAzureStorage(ConfigureAzureStorageArgs),
    #[cfg(feature = "azure")]
    TestAzureConnection,
    #[cfg(feature = "azure")]
    EnableAzureStorage,
    #[cfg(feature = "azure")]
    DisableAzureStorage,
    #[cfg(feature = "azure")]
    ToggleIntranetServer(ToggleIntranetServerArgs),
    #[cfg(feature = "azure")]
    AzureStatus,
}

#[derive(Debug, Args)]
struct CrawlArgs {
    #[arg(long)]
    url: String,
    #[arg(long)]
    output_dir: String,
    #[arg(long)]
    circuits: Option<usize>,
    #[arg(long)]
    download: bool,
    #[arg(long)]
    resume: bool,
    #[arg(long)]
    resume_index: Option<String>,
    #[arg(long)]
    mega_password: Option<String>,
    #[arg(long)]
    agnostic_state: bool,
    #[arg(long)]
    no_listing: bool,
    #[arg(long)]
    no_sizes: bool,
    #[arg(long, hide = true)]
    no_stealth_ramp: bool,
    #[arg(long)]
    force_clearnet: bool,
    /// Download files in parallel while crawl is still running (real-time stream)
    #[arg(long)]
    parallel_download: bool,
    /// Phase 133: Download speed mode — controls circuit caps and pipeline width
    #[arg(long, value_enum, default_value_t = DownloadModeCli::Medium)]
    download_mode: DownloadModeCli,
}

/// Phase 133: CLI-facing enum for --download-mode (maps to frontier::DownloadMode)
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum DownloadModeCli {
    /// Stealth — fewer circuits, minimal server footprint (6 circuits)
    Low,
    /// Balanced — proven 4.75 MB/s over Tor (12 circuits, Phase 132 baseline)
    Medium,
    /// Max throughput — aggressive mirror striping (24 circuits)
    Aggressive,
}

impl From<DownloadModeCli> for DownloadMode {
    fn from(cli: DownloadModeCli) -> Self {
        match cli {
            DownloadModeCli::Low => DownloadMode::Low,
            DownloadModeCli::Medium => DownloadMode::Medium,
            DownloadModeCli::Aggressive => DownloadMode::Aggressive,
        }
    }
}

#[derive(Debug, Args)]
struct DownloadFilesArgs {
    #[arg(long)]
    entries_file: PathBuf,
    #[arg(long)]
    output_dir: String,
    #[arg(long)]
    connections: Option<usize>,
}

#[derive(Debug, Args)]
struct DownloadAllArgs {
    #[arg(long)]
    output_dir: String,
    #[arg(long)]
    connections: Option<usize>,
    #[arg(long)]
    target_url: Option<String>,
    #[arg(long)]
    entries_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
struct InitiateDownloadArgs {
    #[arg(long)]
    url: String,
    #[arg(long)]
    path: String,
    #[arg(long)]
    output_root: String,
    #[arg(long, default_value_t = 8)]
    connections: usize,
    #[arg(long)]
    force_tor: bool,
}

#[derive(Debug, Args)]
struct ExportJsonArgs {
    #[arg(long)]
    output_path: String,
    #[command(flatten)]
    source: SnapshotSourceArgs,
}

#[derive(Debug, Args)]
struct GetVfsChildrenArgs {
    #[arg(long)]
    parent_path: String,
    #[command(flatten)]
    source: SnapshotSourceArgs,
}

#[derive(Debug, Clone, Args)]
struct SnapshotSourceArgs {
    #[arg(long)]
    entries_file: Option<PathBuf>,
    #[arg(long)]
    source_target_url: Option<String>,
    #[arg(long)]
    source_output_dir: Option<String>,
}

#[derive(Debug, Args)]
struct IngestVfsEntriesArgs {
    #[arg(long)]
    entries_file: PathBuf,
}

#[derive(Debug, Args)]
struct PreResolveArgs {
    #[arg(long)]
    url: String,
}

#[derive(Debug, Args)]
struct SubtreeHeatmapArgs {
    #[arg(long)]
    target_key: String,
}

#[derive(Debug, Args)]
struct OpenFolderArgs {
    #[arg(long)]
    path: String,
}

#[derive(Debug, Args)]
struct SetTelemetryEnabledArgs {
    #[arg(long, action = ArgAction::Set)]
    enabled: bool,
}

#[derive(Debug, Args)]
struct DrainTelemetryRingArgs {
    #[arg(long, value_enum, default_value_t = ByteFormat::Base64)]
    format: ByteFormat,
}

#[derive(Debug, Args)]
struct DetectInputModeArgs {
    #[arg(long)]
    input: String,
}

#[derive(Debug, Args)]
struct FetchNetworkDiskBlockArgs {
    #[arg(long)]
    url: String,
    #[arg(long)]
    lba: u64,
    #[arg(long, default_value_t = 4096)]
    block_size: usize,
    #[arg(long, value_enum, default_value_t = ByteFormat::Base64)]
    format: ByteFormat,
}

#[derive(Debug, Args)]
struct FetchNetworkDiskExtentsArgs {
    #[arg(long)]
    url: String,
    #[arg(long, default_value_t = 4096)]
    block_size: usize,
    #[arg(long = "extent", required = true, value_parser = parse_extent_arg)]
    extents: Vec<(u64, usize)>,
    #[arg(long, value_enum, default_value_t = ByteFormat::Base64)]
    format: ByteFormat,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum ByteFormat {
    Base64,
    Hex,
    Json,
}

#[cfg(feature = "azure")]
#[derive(Debug, Args)]
struct ConfigureAzureStorageArgs {
    #[arg(long)]
    subscription_id: String,
    #[arg(long)]
    tenant_id: String,
    #[arg(long)]
    client_id: String,
    #[arg(long, default_value = "")]
    client_secret: String,
    #[arg(long)]
    resource_group: String,
    #[arg(long)]
    storage_account: String,
    #[arg(long, default_value = "crawli-downloads")]
    container_name: String,
    #[arg(long, default_value = "eastus")]
    region: String,
    #[arg(long, default_value_t = 500)]
    size_gb: u32,
    #[arg(long)]
    use_managed_identity: bool,
}

#[cfg(feature = "azure")]
#[derive(Debug, Args)]
struct ToggleIntranetServerArgs {
    #[arg(long, action = ArgAction::Set)]
    enable: bool,
    #[arg(long)]
    port: Option<u16>,
}

fn benchmark_flag_override_enabled() -> bool {
    matches!(
        std::env::var("CRAWLI_ALLOW_BENCHMARK_FLAGS")
            .ok()
            .as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn resolve_stealth_ramp(no_stealth_ramp: bool, allow_benchmark_override: bool) -> bool {
    !(no_stealth_ramp && allow_benchmark_override)
}

impl CrawlArgs {
    fn to_options(&self) -> crate::frontier::CrawlOptions {
        let mode: DownloadMode = self.download_mode.into();
        crate::frontier::CrawlOptions {
            listing: !self.no_listing,
            sizes: !self.no_sizes,
            download: self.download,
            circuits: Some(self.circuits.unwrap_or(mode.default_circuits()).max(1)),
            agnostic_state: self.agnostic_state,
            resume: self.resume || self.resume_index.is_some(),
            resume_index: self.resume_index.clone(),
            mega_password: self.mega_password.clone(),
            stealth_ramp: resolve_stealth_ramp(
                self.no_stealth_ramp,
                benchmark_flag_override_enabled(),
            ),
            force_clearnet: self.force_clearnet,
            parallel_download: self.parallel_download,
            download_mode: mode,
        }
    }
}

impl From<InitiateDownloadArgs> for crate::DownloadArgs {
    fn from(value: InitiateDownloadArgs) -> Self {
        Self {
            url: value.url,
            path: value.path,
            output_root: value.output_root,
            connections: value.connections,
            force_tor: value.force_tor,
        }
    }
}

#[cfg(not(test))]
fn parse_and_run_cli_args(args: Vec<String>) -> i32 {
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(err) => {
            let exit_code = match err.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
                _ => 2,
            };
            let _ = err.print();
            return exit_code;
        }
    };

    if cli.command.is_none() {
        let mut cmd = Cli::command();
        let _ = cmd.print_help();
        println!();
        return 0;
    }

    match run_cli(cli) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("CLI error: {err}");
            1
        }
    }
}

#[cfg(not(test))]
pub(crate) fn try_run_from_env() -> Option<i32> {
    let args = std::env::args().collect::<Vec<_>>();
    if !should_run_cli_mode(&args) {
        return None;
    }

    Some(parse_and_run_cli_args(args))
}

#[cfg(not(test))]
pub(crate) fn run_cli_from_env() -> i32 {
    let args = std::env::args().collect::<Vec<_>>();
    parse_and_run_cli_args(args)
}

#[cfg(test)]
pub(crate) fn run_cli_from_env() -> i32 {
    0
}

#[cfg(test)]
pub(crate) fn try_run_from_env() -> Option<i32> {
    None
}

fn should_run_cli_mode(args: &[String]) -> bool {
    args.len() > 1 && !args[1].starts_with("-psn_")
}

#[cfg(not(test))]
fn run_cli(cli: Cli) -> Result<(), String> {
    let mut ctx = crate::tauri_context();
    ctx.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .manage(crate::AppState::default())
        .build(ctx)
        .map_err(|e| format!("Failed to build headless Tauri app: {e}"))?;
    let app_handle = app.handle().clone();
    let state = app.state::<crate::AppState>();
    let telemetry = state.telemetry.clone();
    let bridge = state.telemetry_bridge.clone();

    if !cli.quiet_events {
        install_cli_event_streams(
            &app_handle,
            CliEventConfig {
                include_telemetry_events: cli.include_telemetry_events,
                progress_summary: cli.progress_summary,
                progress_summary_interval_ms: cli.progress_summary_interval_ms.max(250),
            },
        );
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .map_err(|e| format!("Failed to build Tokio runtime: {e}"))?;

    let compact_json = cli.compact_json;
    let command = cli
        .command
        .ok_or_else(|| "No CLI command was provided.".to_string())?;

    let result = runtime.block_on(async move {
        crate::runtime_metrics::spawn_metrics_emitter(app_handle.clone(), telemetry);
        crate::telemetry_bridge::spawn_bridge_emitter(app_handle.clone(), bridge);

        let value = dispatch_command(app_handle.clone(), command).await?;
        write_cli_value(&value, compact_json)?;
        Ok::<(), String>(())
    });

    drop(runtime);
    drop(app);
    crate::tor::cleanup_stale_tor_daemons();
    result
}

#[cfg(not(test))]
struct CliEventConfig {
    include_telemetry_events: bool,
    progress_summary: bool,
    progress_summary_interval_ms: u64,
}

#[allow(dead_code)]
#[derive(Default)]
struct CliProgressSummaryState {
    crawl_status: Option<crate::telemetry_bridge::BridgeCrawlStatus>,
    resource_metrics: Option<crate::runtime_metrics::ResourceMetricsSnapshot>,
    batch_progress: Option<crate::telemetry_bridge::BridgeBatchProgress>,
    download_progress: Option<crate::telemetry_bridge::BridgeDownloadProgress>,
    last_rendered: Option<String>,
    last_final_rendered: Option<String>,
    last_emitted_at: Option<std::time::Instant>,
    last_rss_emitted_at: Option<std::time::Instant>,
}

#[cfg(not(test))]
fn install_cli_event_streams(app: &tauri::AppHandle, config: CliEventConfig) {
    const RAW_EVENTS: &[&str] = &["log", "crawl_log"];
    let mut structured_events = vec![
        "tor_status",
        "crawl_status_update",
        "download_batch_started",
        "download_resume_plan",
        "download_failed",
        "download_interrupted",
        "qilin_nodes_updated",
    ];
    if config.include_telemetry_events {
        structured_events.push("telemetry_bridge_update");
    }

    for event_name in RAW_EVENTS {
        let label = (*event_name).to_string();
        app.listen(*event_name, move |event| {
            eprintln!("[{label}] {}", normalize_event_payload(event.payload()));
        });
    }

    for event_name in structured_events {
        let label = event_name.to_string();
        app.listen(event_name, move |event| {
            eprintln!("[event:{label}] {}", event.payload());
        });
    }

    if config.progress_summary {
        let state = Arc::new(Mutex::new(CliProgressSummaryState::default()));
        let interval = std::time::Duration::from_millis(config.progress_summary_interval_ms);
        app.listen("telemetry_bridge_update", move |event| {
            let Ok(update) = serde_json::from_str::<crate::telemetry_bridge::TelemetryBridgeUpdate>(
                event.payload(),
            ) else {
                return;
            };

            let mut guard = match state.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };

            if let Some(crawl_status) = update.crawl_status {
                guard.crawl_status = Some(crawl_status);
            }
            if let Some(resource_metrics) = update.resource_metrics {
                guard.resource_metrics = Some(resource_metrics);
            }
            if let Some(batch_progress) = update.batch_progress {
                guard.batch_progress = Some(batch_progress);
            }
            if !update.download_progress.is_empty() {
                guard.download_progress = update
                    .download_progress
                    .into_iter()
                    .max_by_key(|progress| progress.bytes_downloaded);
            }

            let rendered = render_progress_summary(&guard);
            let now = std::time::Instant::now();
            let force_emit = guard.crawl_status.as_ref().is_some_and(|status| {
                matches!(status.phase.as_str(), "complete" | "cancelled" | "error")
            });
            let enough_time = guard
                .last_emitted_at
                .is_none_or(|last| now.duration_since(last) >= interval);
            let changed = guard.last_rendered.as_ref() != Some(&rendered);

            if force_emit {
                let final_rendered = render_final_progress_summary(&guard);
                if guard.last_final_rendered.as_ref() != Some(&final_rendered) {
                    eprintln!("[summary:final] {final_rendered}");
                    guard.last_final_rendered = Some(final_rendered);
                }
                guard.last_rendered = Some(rendered);
                guard.last_emitted_at = Some(now);
            } else if changed && enough_time {
                eprintln!("[summary] {rendered}");
                guard.last_rendered = Some(rendered);
                guard.last_emitted_at = Some(now);
            }

            if let Some(metrics) = &guard.resource_metrics {
                let rss_enough_time = guard
                    .last_rss_emitted_at
                    .is_none_or(|last| now.duration_since(last).as_secs() >= 15);

                if rss_enough_time {
                    let rss_mb = metrics.process_memory_bytes as f64 / 1_048_576.0;
                    eprintln!("[telemetry:rss] Memory RSS: {:.1} MB", rss_mb);
                    guard.last_rss_emitted_at = Some(now);
                }
            }
        });
    }
}

#[cfg(not(test))]
fn normalize_event_payload(payload: &str) -> String {
    serde_json::from_str::<String>(payload).unwrap_or_else(|_| payload.to_string())
}

fn format_summary_mebibytes(bytes: u64) -> String {
    format!("{:.1}MB", bytes as f64 / 1_048_576.0)
}

fn render_progress_summary(state: &CliProgressSummaryState) -> String {
    let crawl = state.crawl_status.as_ref();
    let metrics = state.resource_metrics.as_ref();
    let batch = state.batch_progress.as_ref();
    let download = state.download_progress.as_ref();

    let phase = crawl.map(|status| status.phase.as_str()).unwrap_or("idle");
    let progress = crawl.map(|status| status.progress_percent).unwrap_or(0.0);
    let visited = crawl.map(|status| status.visited_nodes).unwrap_or(0);
    let processed = crawl.map(|status| status.processed_nodes).unwrap_or(0);
    let queued = crawl.map(|status| status.queued_nodes).unwrap_or(0);
    let delta = crawl.map(|status| status.delta_new_files).unwrap_or(0);
    let active_workers = metrics
        .filter(|snapshot| snapshot.worker_target > 0)
        .map(|snapshot| snapshot.active_workers)
        .or_else(|| crawl.map(|status| status.active_workers))
        .unwrap_or(0);
    let worker_target = metrics
        .filter(|snapshot| snapshot.worker_target > 0)
        .map(|snapshot| snapshot.worker_target)
        .or_else(|| crawl.map(|status| status.worker_target))
        .unwrap_or(0);
    let active_circuits = metrics
        .map(|snapshot| snapshot.active_circuits)
        .unwrap_or(0);
    let failovers = metrics.map(|snapshot| snapshot.node_failovers).unwrap_or(0);
    let throttles = metrics.map(|snapshot| snapshot.throttle_count).unwrap_or(0);
    let timeouts = metrics.map(|snapshot| snapshot.timeout_count).unwrap_or(0);
    let node = metrics
        .and_then(|snapshot| snapshot.current_node_host.as_deref())
        .map(shorten_host_for_summary)
        .unwrap_or_else(|| "-".to_string());
    let download_host_cache_hits = metrics
        .map(|snapshot| snapshot.download_host_cache_hits)
        .unwrap_or(0);
    let download_probe_promotion_hits = metrics
        .map(|snapshot| snapshot.download_probe_promotion_hits)
        .unwrap_or(0);
    let download_low_speed_aborts = metrics
        .map(|snapshot| snapshot.download_low_speed_aborts)
        .unwrap_or(0);
    let download_probe_quarantine_hits = metrics
        .map(|snapshot| snapshot.download_probe_quarantine_hits)
        .unwrap_or(0);
    let download_probe_candidate_exhaustions = metrics
        .map(|snapshot| snapshot.download_probe_candidate_exhaustions)
        .unwrap_or(0);
    let qilin_fresh_redirect_candidates = metrics
        .map(|snapshot| snapshot.qilin_fresh_redirect_candidates)
        .unwrap_or(0);
    let qilin_stale_host_only_candidates = metrics
        .map(|snapshot| snapshot.qilin_stale_host_only_candidates)
        .unwrap_or(0);
    let qilin_degraded_stage_d_activations = metrics
        .map(|snapshot| snapshot.qilin_degraded_stage_d_activations)
        .unwrap_or(0);
    let eta = crawl
        .and_then(|status| status.eta_seconds)
        .map(|seconds| format!("{seconds}s"))
        .unwrap_or_else(|| "-".to_string());

    let mut summary = format!(
        "phase={phase} progress={progress:.1}% seen={visited} processed={processed} queue={queued} workers={active_workers}/{worker_target} delta={delta} node={node} circuits={active_circuits} failovers={failovers} 429/503={throttles} timeouts={timeouts} dl_transport={download_host_cache_hits}/{download_probe_promotion_hits}/{download_low_speed_aborts} probe_admission={download_probe_quarantine_hits}/{download_probe_candidate_exhaustions} qilin_discovery=fresh:{qilin_fresh_redirect_candidates} stale:{qilin_stale_host_only_candidates} degraded:{qilin_degraded_stage_d_activations} eta={eta}"
    );

    if let Some(batch_progress) = batch {
        summary.push_str(&format!(
            " download={}/{} failed={} speed={:.2}MB/s bytes={} file={}",
            batch_progress.completed,
            batch_progress.total,
            batch_progress.failed,
            batch_progress.speed_mbps,
            format_summary_mebibytes(batch_progress.downloaded_bytes),
            trim_summary_path(&batch_progress.current_file, 48),
        ));
    } else if let Some(download_progress) = download {
        let total = download_progress
            .total_bytes
            .map(format_summary_mebibytes)
            .unwrap_or_else(|| "-".to_string());
        summary.push_str(&format!(
            " single={}/{} speed={:.2}MB/s file={}",
            format_summary_mebibytes(download_progress.bytes_downloaded),
            total,
            download_progress.speed_bps as f64 / 1_048_576.0,
            trim_summary_path(&download_progress.path, 48),
        ));
    }

    summary
}

fn render_final_progress_summary(state: &CliProgressSummaryState) -> String {
    let crawl = state.crawl_status.as_ref();
    let metrics = state.resource_metrics.as_ref();
    let batch = state.batch_progress.as_ref();
    let download = state.download_progress.as_ref();

    let phase = crawl.map(|status| status.phase.as_str()).unwrap_or("idle");
    let visited = crawl.map(|status| status.visited_nodes).unwrap_or(0);
    let processed = crawl.map(|status| status.processed_nodes).unwrap_or(0);
    let queued = crawl.map(|status| status.queued_nodes).unwrap_or(0);
    let active_workers = metrics
        .filter(|snapshot| snapshot.worker_target > 0)
        .map(|snapshot| snapshot.active_workers)
        .or_else(|| crawl.map(|status| status.active_workers))
        .unwrap_or(0);
    let worker_target = metrics
        .filter(|snapshot| snapshot.worker_target > 0)
        .map(|snapshot| snapshot.worker_target)
        .or_else(|| crawl.map(|status| status.worker_target))
        .unwrap_or(0);
    let node = metrics
        .and_then(|snapshot| snapshot.current_node_host.as_deref())
        .map(shorten_host_for_summary)
        .unwrap_or_else(|| "-".to_string());
    let requests = metrics.map(|snapshot| snapshot.total_requests).unwrap_or(0);
    let successful_requests = metrics
        .map(|snapshot| snapshot.successful_requests)
        .unwrap_or(0);
    let failed_requests = metrics
        .map(|snapshot| snapshot.failed_requests)
        .unwrap_or(0);
    let subtree_reroutes = metrics
        .map(|snapshot| snapshot.subtree_reroutes)
        .unwrap_or(0);
    let subtree_quarantine_hits = metrics
        .map(|snapshot| snapshot.subtree_quarantine_hits)
        .unwrap_or(0);
    let off_winner_child_requests = metrics
        .map(|snapshot| snapshot.off_winner_child_requests)
        .unwrap_or(0);
    let winner_host = metrics
        .and_then(|snapshot| snapshot.winner_host.as_deref())
        .or_else(|| metrics.and_then(|snapshot| snapshot.current_node_host.as_deref()))
        .map(shorten_host_for_summary)
        .unwrap_or_else(|| "-".to_string());
    let slowest_circuit = metrics
        .and_then(|snapshot| snapshot.slowest_circuit.as_deref())
        .unwrap_or("-");
    let late_throttles = metrics.map(|snapshot| snapshot.late_throttles).unwrap_or(0);
    let outlier_isolations = metrics
        .map(|snapshot| snapshot.outlier_isolations)
        .unwrap_or(0);
    let download_host_cache_hits = metrics
        .map(|snapshot| snapshot.download_host_cache_hits)
        .unwrap_or(0);
    let download_probe_promotion_hits = metrics
        .map(|snapshot| snapshot.download_probe_promotion_hits)
        .unwrap_or(0);
    let download_low_speed_aborts = metrics
        .map(|snapshot| snapshot.download_low_speed_aborts)
        .unwrap_or(0);
    let download_probe_quarantine_hits = metrics
        .map(|snapshot| snapshot.download_probe_quarantine_hits)
        .unwrap_or(0);
    let download_probe_candidate_exhaustions = metrics
        .map(|snapshot| snapshot.download_probe_candidate_exhaustions)
        .unwrap_or(0);
    let qilin_fresh_redirect_candidates = metrics
        .map(|snapshot| snapshot.qilin_fresh_redirect_candidates)
        .unwrap_or(0);
    let qilin_stale_host_only_candidates = metrics
        .map(|snapshot| snapshot.qilin_stale_host_only_candidates)
        .unwrap_or(0);
    let qilin_degraded_stage_d_activations = metrics
        .map(|snapshot| snapshot.qilin_degraded_stage_d_activations)
        .unwrap_or(0);

    let mut summary = format!(
        "phase={phase} seen={visited} processed={processed} queue={queued} workers={active_workers}/{worker_target} node={node} req={requests}/{successful_requests}/{failed_requests} subtree={subtree_reroutes}/{subtree_quarantine_hits}/{off_winner_child_requests} dl_transport={download_host_cache_hits}/{download_probe_promotion_hits}/{download_low_speed_aborts} probe_admission={download_probe_quarantine_hits}/{download_probe_candidate_exhaustions} qilin_discovery=fresh:{qilin_fresh_redirect_candidates} stale:{qilin_stale_host_only_candidates} degraded:{qilin_degraded_stage_d_activations} tail={winner_host}/{slowest_circuit}/{late_throttles}/{outlier_isolations}"
    );
    if let Some(batch_progress) = batch {
        summary.push_str(&format!(
            " download={}/{} failed={} speed={:.2}MB/s bytes={}",
            batch_progress.completed,
            batch_progress.total,
            batch_progress.failed,
            batch_progress.speed_mbps,
            format_summary_mebibytes(batch_progress.downloaded_bytes),
        ));
    } else if let Some(download_progress) = download {
        let total = download_progress
            .total_bytes
            .map(format_summary_mebibytes)
            .unwrap_or_else(|| "-".to_string());
        summary.push_str(&format!(
            " single={}/{} speed={:.2}MB/s",
            format_summary_mebibytes(download_progress.bytes_downloaded),
            total,
            download_progress.speed_bps as f64 / 1_048_576.0,
        ));
    }
    summary
}

fn shorten_host_for_summary(host: &str) -> String {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return "-".to_string();
    }
    if trimmed.len() <= 24 {
        trimmed.to_string()
    } else {
        format!("{}...{}", &trimmed[..12], &trimmed[trimmed.len() - 8..])
    }
}

fn trim_summary_path(value: &str, max_len: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= max_len || max_len <= 3 {
        trimmed.to_string()
    } else {
        format!("...{}", &trimmed[trimmed.len() - (max_len - 3)..])
    }
}

#[cfg(not(test))]
async fn dispatch_command(
    app: tauri::AppHandle,
    command: CliCommand,
) -> Result<serde_json::Value, String> {
    match command {
        CliCommand::Crawl(args) => {
            let options = args.to_options();
            let result = crate::start_crawl(args.url, options, args.output_dir, app).await?;
            to_json_value(result)
        }
        CliCommand::DownloadFiles(args) => {
            let mut entries = read_entries_from_file(&args.entries_file).await?;
            crate::maybe_repin_qilin_entries_from_context(
                &mut entries,
                Some(args.entries_file.as_path()),
                &app,
            )
            .map_err(|e| e.to_string())?;
            let written =
                crate::download_files(entries, args.output_dir, args.connections, app).await?;
            Ok(json!({ "writtenCount": written }))
        }
        CliCommand::DownloadAll(args) => {
            if args.entries_file.is_some() && args.target_url.is_some() {
                return Err(
                    "Use either --entries-file or --target-url for download-all, not both."
                        .to_string(),
                );
            }
            if let Some(entries_file) = &args.entries_file {
                let mut entries = read_entries_from_file(entries_file).await?;
                crate::maybe_repin_qilin_entries_from_context(
                    &mut entries,
                    Some(entries_file.as_path()),
                    &app,
                )
                .map_err(|e| e.to_string())?;
                let summary = hydrate_vfs_from_entries(&app, &entries, "download_all").await?;
                let written =
                    crate::download_all(args.output_dir, args.connections, None, app).await?;
                Ok(json!({
                    "writtenCount": written,
                    "hydratedSummary": summary,
                }))
            } else {
                let written =
                    crate::download_all(args.output_dir, args.connections, args.target_url, app)
                        .await?;
                Ok(json!({ "writtenCount": written }))
            }
        }
        CliCommand::InitiateDownload(args) => {
            let download_args: crate::DownloadArgs = args.into();
            crate::run_single_download_blocking(app, download_args).await?;
            Ok(json!({ "status": "completed" }))
        }
        CliCommand::CancelCrawl => {
            let message = crate::cancel_crawl(app).await?;
            Ok(json!({ "message": message }))
        }
        CliCommand::PauseDownload => {
            let paused = crate::pause_active_download(app)?;
            Ok(json!({ "paused": paused }))
        }
        CliCommand::StopDownload => {
            let stopped = crate::stop_active_download(app)?;
            Ok(json!({ "stopped": stopped }))
        }
        CliCommand::ExportJson(args) => {
            let entries = load_entries_from_source(&args.source).await?;
            let summary = hydrate_vfs_from_entries(&app, &entries, "export_json").await?;
            let message = crate::export_json(args.output_path, app).await?;
            Ok(json!({ "message": message, "hydratedSummary": summary }))
        }
        CliCommand::GetVfsChildren(args) => {
            let entries = load_entries_from_source(&args.source).await?;
            let summary = hydrate_vfs_from_entries(&app, &entries, "get_vfs_children").await?;
            let children = crate::get_vfs_children(args.parent_path, app).await?;
            Ok(json!({
                "hydratedSummary": summary,
                "children": children,
            }))
        }
        CliCommand::AdapterCatalog => to_json_value(crate::get_adapter_support_catalog()),
        CliCommand::IngestVfsEntries(args) => {
            let entries = read_entries_from_file(&args.entries_file).await?;
            let summary = hydrate_vfs_from_entries(&app, &entries, "ingest_vfs_entries").await?;
            Ok(json!({
                "ingestedEntries": entries.len(),
                "summary": summary,
            }))
        }
        CliCommand::PreResolve(args) => {
            crate::perform_pre_resolve_onion(&args.url, &app).await?;
            Ok(json!({ "status": "resolved", "url": args.url }))
        }
        CliCommand::SubtreeHeatmap(args) => crate::get_subtree_heatmap(args.target_key).await,
        CliCommand::OpenFolder(args) => {
            crate::open_folder_os(args.path.clone()).await?;
            Ok(json!({ "opened": args.path }))
        }
        CliCommand::SetTelemetryEnabled(args) => {
            crate::set_telemetry_enabled(args.enabled);
            Ok(json!({ "telemetryEnabled": args.enabled }))
        }
        CliCommand::DrainTelemetryRing(args) => {
            let payload = crate::binary_telemetry::drain_telemetry_ring();
            Ok(format_bytes_payload(payload, args.format))
        }
        CliCommand::DetectInputMode(args) => {
            let mode = crate::detect_input_mode(args.input.clone());
            Ok(json!({ "input": args.input, "mode": mode }))
        }
        CliCommand::SystemProfile => Ok(crate::get_system_profile()),
        CliCommand::FetchNetworkDiskBlock(args) => {
            let bytes = crate::network_disk::fetch_network_disk_block_cmd(
                args.url,
                args.lba,
                args.block_size,
            )
            .await?;
            Ok(format_bytes_payload(bytes, args.format))
        }
        CliCommand::FetchNetworkDiskExtents(args) => {
            let bytes = crate::network_disk::fetch_network_disk_extents_cmd(
                args.url,
                args.block_size,
                args.extents,
            )
            .await?;
            Ok(format_bytes_payload(bytes, args.format))
        }
        #[cfg(feature = "azure")]
        CliCommand::ConfigureAzureStorage(args) => {
            let config = crate::azure_connectivity::AzureStorageConfig {
                subscription_id: args.subscription_id,
                tenant_id: args.tenant_id,
                client_id: args.client_id,
                client_secret_encrypted: args.client_secret,
                resource_group: args.resource_group,
                storage_account: args.storage_account,
                container_name: args.container_name,
                region: args.region,
                size_gb: args.size_gb,
                use_managed_identity: args.use_managed_identity,
            };
            let message = crate::azure_connectivity::configure_azure_storage(config, app).await?;
            Ok(json!({ "message": message }))
        }
        #[cfg(feature = "azure")]
        CliCommand::TestAzureConnection => {
            let message = crate::azure_connectivity::test_azure_connection(app).await?;
            Ok(json!({ "message": message }))
        }
        #[cfg(feature = "azure")]
        CliCommand::EnableAzureStorage => {
            let message = crate::azure_connectivity::enable_azure_storage(app).await?;
            Ok(json!({ "message": message }))
        }
        #[cfg(feature = "azure")]
        CliCommand::DisableAzureStorage => {
            let message = crate::azure_connectivity::disable_azure_storage(app).await?;
            Ok(json!({ "message": message }))
        }
        #[cfg(feature = "azure")]
        CliCommand::ToggleIntranetServer(args) => {
            let message =
                crate::azure_connectivity::toggle_intranet_server(args.enable, args.port, app)
                    .await?;
            Ok(json!({ "message": message }))
        }
        #[cfg(feature = "azure")]
        CliCommand::AzureStatus => crate::azure_connectivity::get_azure_status(app).await,
    }
}

#[cfg(not(test))]
async fn read_entries_from_file(path: &Path) -> Result<Vec<crate::adapters::FileEntry>, String> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read entries file {}: {e}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse entries JSON {}: {e}", path.display()))
}

#[cfg(not(test))]
async fn load_entries_from_source(
    source: &SnapshotSourceArgs,
) -> Result<Vec<crate::adapters::FileEntry>, String> {
    match (
        source.entries_file.as_ref(),
        source.source_target_url.as_ref(),
        source.source_output_dir.as_ref(),
    ) {
        (Some(path), None, None) => read_entries_from_file(path).await,
        (None, Some(target_url), Some(output_dir)) => {
            load_entries_from_target_snapshot(target_url, output_dir).await
        }
        (None, Some(_), None) => Err(
            "When using --source-target-url you must also provide --source-output-dir."
                .to_string(),
        ),
        (Some(_), Some(_), _) => Err(
            "Choose either --entries-file or the --source-target-url/--source-output-dir pair."
                .to_string(),
        ),
        _ => Err(
            "No source data provided. Use --entries-file or --source-target-url with --source-output-dir."
                .to_string(),
        ),
    }
}

#[cfg(not(test))]
async fn load_entries_from_target_snapshot(
    target_url: &str,
    output_dir: &str,
) -> Result<Vec<crate::adapters::FileEntry>, String> {
    let output_root = crate::canonical_output_root(output_dir)?;
    let target_paths =
        crate::target_state::target_paths(&output_root, target_url).map_err(|e| e.to_string())?;

    let best_entries = crate::target_state::load_entries_snapshot(&target_paths.best_snapshot_path)
        .map_err(|e| e.to_string())?;
    if !best_entries.is_empty() {
        return Ok(best_entries);
    }

    let current_entries =
        crate::target_state::load_entries_snapshot(&target_paths.current_snapshot_path)
            .map_err(|e| e.to_string())?;
    if !current_entries.is_empty() {
        return Ok(current_entries);
    }

    Err(format!(
        "No saved crawl snapshot found for target {} under {}.",
        target_url,
        output_root.display()
    ))
}

#[cfg(not(test))]
async fn hydrate_vfs_from_entries(
    app: &tauri::AppHandle,
    entries: &[crate::adapters::FileEntry],
    label: &str,
) -> Result<crate::db::VfsSummary, String> {
    let vfs_path = std::env::temp_dir().join(format!(
        "crawli_cli_vfs_{}_{}_{}",
        sanitize_cli_label(label),
        std::process::id(),
        unix_timestamp_ms()
    ));
    let vfs_path_str = vfs_path.to_string_lossy().to_string();
    let state = app.state::<crate::AppState>();
    state
        .vfs
        .initialize(&vfs_path_str)
        .await
        .map_err(|e| format!("Failed to initialize CLI VFS {}: {e}", vfs_path.display()))?;
    state
        .vfs
        .clear()
        .await
        .map_err(|e| format!("Failed to clear CLI VFS {}: {e}", vfs_path.display()))?;
    crate::ingest_vfs_entries(entries.to_vec(), app.clone()).await?;
    Ok(crate::summarize_entry_slice(entries))
}

#[cfg(not(test))]
fn sanitize_cli_label(label: &str) -> String {
    label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(not(test))]
fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn parse_extent_arg(spec: &str) -> Result<(u64, usize), String> {
    let (start, count) = spec
        .split_once(':')
        .ok_or_else(|| "Extent must use the form <lba>:<byte_count>.".to_string())?;
    let lba = start
        .trim()
        .parse::<u64>()
        .map_err(|e| format!("Invalid extent LBA '{}': {e}", start.trim()))?;
    let byte_count = count
        .trim()
        .parse::<usize>()
        .map_err(|e| format!("Invalid extent byte count '{}': {e}", count.trim()))?;
    Ok((lba, byte_count))
}

#[cfg(not(test))]
fn format_bytes_payload(bytes: Vec<u8>, format: ByteFormat) -> serde_json::Value {
    match format {
        ByteFormat::Base64 => json!({
            "encoding": "base64",
            "byteCount": bytes.len(),
            "data": BASE64_STANDARD.encode(bytes),
        }),
        ByteFormat::Hex => json!({
            "encoding": "hex",
            "byteCount": bytes.len(),
            "data": hex::encode(bytes),
        }),
        ByteFormat::Json => json!({
            "encoding": "json",
            "byteCount": bytes.len(),
            "data": bytes,
        }),
    }
}

#[cfg(not(test))]
fn to_json_value<T: serde::Serialize>(value: T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[cfg(not(test))]
fn write_cli_value(value: &serde_json::Value, compact_json: bool) -> Result<(), String> {
    let rendered = if compact_json {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    }
    .map_err(|e| e.to_string())?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_boot_arg_keeps_gui_mode() {
        assert!(!should_run_cli_mode(&[
            "crawli".to_string(),
            "-psn_0_12345".to_string()
        ]));
    }

    #[test]
    fn explicit_subcommand_enters_cli_mode() {
        assert!(should_run_cli_mode(&[
            "crawli".to_string(),
            "crawl".to_string()
        ]));
    }

    #[test]
    fn extent_parser_requires_colon_pair() {
        assert_eq!(parse_extent_arg("12:4096").unwrap(), (12, 4096));
        assert!(parse_extent_arg("12-4096").is_err());
    }

    #[test]
    fn crawl_defaults_match_gui_surface() {
        let args = CrawlArgs {
            url: "http://example.onion".to_string(),
            output_dir: "/tmp/out".to_string(),
            circuits: None,
            download: false,
            resume: false,
            resume_index: None,
            mega_password: None,
            agnostic_state: false,
            no_listing: false,
            no_sizes: false,
            no_stealth_ramp: false,
            force_clearnet: false,
            parallel_download: false,
            download_mode: DownloadModeCli::Medium,
        };
        let options = args.to_options();
        assert!(options.listing);
        assert!(options.sizes);
        assert!(!options.download);
        assert!(!options.agnostic_state);
        assert!(options.stealth_ramp);
    }

    #[test]
    fn no_stealth_ramp_stays_benchmark_only_without_override() {
        assert!(resolve_stealth_ramp(true, false));
        assert!(!resolve_stealth_ramp(true, true));
        assert!(resolve_stealth_ramp(false, true));
    }

    #[test]
    fn progress_summary_prefers_resource_worker_metrics() {
        let state = CliProgressSummaryState {
            crawl_status: Some(crate::telemetry_bridge::BridgeCrawlStatus {
                phase: "crawling".to_string(),
                progress_percent: 42.5,
                visited_nodes: 120,
                processed_nodes: 80,
                queued_nodes: 40,
                active_workers: 2,
                worker_target: 4,
                eta_seconds: Some(90),
                estimation: "adaptive-frontier".to_string(),
                delta_new_files: 77,
                vanguard: None,
            }),
            resource_metrics: Some(crate::runtime_metrics::ResourceMetricsSnapshot {
                active_workers: 6,
                worker_target: 12,
                active_circuits: 3,
                current_node_host: Some(
                    "bw2eqn5sp5yhe64gfbodbrepo6jyicsbkcl47e6knlenctrwh5gwn3ad.onion".to_string(),
                ),
                node_failovers: 1,
                throttle_count: 2,
                timeout_count: 3,
                download_host_cache_hits: 5,
                download_probe_promotion_hits: 2,
                download_low_speed_aborts: 1,
                download_probe_quarantine_hits: 4,
                download_probe_candidate_exhaustions: 1,
                qilin_fresh_redirect_candidates: 2,
                qilin_stale_host_only_candidates: 17,
                qilin_degraded_stage_d_activations: 1,
                ..Default::default()
            }),
            batch_progress: Some(crate::telemetry_bridge::BridgeBatchProgress {
                completed: 25,
                failed: 1,
                total: 100,
                current_file: "/tmp/downloads/example.pdf".to_string(),
                speed_mbps: 2.75,
                downloaded_bytes: 52 * 1_048_576,
                active_circuits: Some(4),
                bbr_bottleneck_mbps: None,
                ekf_covariance: None,
            }),
            download_progress: None,
            last_rendered: None,
            last_final_rendered: None,
            last_emitted_at: None,
            last_rss_emitted_at: None,
        };

        let rendered = render_progress_summary(&state);

        assert!(rendered.contains("phase=crawling"));
        assert!(rendered.contains("workers=6/12"));
        assert!(rendered.contains("failovers=1"));
        assert!(rendered.contains("429/503=2"));
        assert!(rendered.contains("timeouts=3"));
        assert!(rendered.contains("dl_transport=5/2/1"));
        assert!(rendered.contains("probe_admission=4/1"));
        assert!(rendered.contains("qilin_discovery=fresh:2 stale:17 degraded:1"));
        assert!(rendered.contains("eta=90s"));
        assert!(rendered.contains("download=25/100 failed=1"));
        assert!(rendered.contains("speed=2.75MB/s"));
        assert!(rendered.contains("bytes=52.0MB"));
    }

    #[test]
    fn final_progress_summary_includes_route_counters() {
        let state = CliProgressSummaryState {
            crawl_status: Some(crate::telemetry_bridge::BridgeCrawlStatus {
                phase: "complete".to_string(),
                progress_percent: 100.0,
                visited_nodes: 544,
                processed_nodes: 282,
                queued_nodes: 0,
                active_workers: 0,
                worker_target: 16,
                eta_seconds: None,
                estimation: "complete".to_string(),
                delta_new_files: 0,
                vanguard: None,
            }),
            resource_metrics: Some(crate::runtime_metrics::ResourceMetricsSnapshot {
                active_workers: 0,
                worker_target: 16,
                current_node_host: Some(
                    "chygwjfxnehjkisuex7crh6mqlfbjs2cbr6drskdrf4gy4yyxbpcbsyd.onion".to_string(),
                ),
                winner_host: Some(
                    "winnerqualityhostqs2x2f6pao7fdksm4rwr5iw65nq7w4g4n5.onion".to_string(),
                ),
                slowest_circuit: Some("c7:8450ms".to_string()),
                total_requests: 87,
                successful_requests: 79,
                failed_requests: 8,
                subtree_reroutes: 3,
                subtree_quarantine_hits: 2,
                off_winner_child_requests: 0,
                late_throttles: 2,
                outlier_isolations: 1,
                download_host_cache_hits: 9,
                download_probe_promotion_hits: 4,
                download_low_speed_aborts: 2,
                download_probe_quarantine_hits: 7,
                download_probe_candidate_exhaustions: 3,
                qilin_fresh_redirect_candidates: 0,
                qilin_stale_host_only_candidates: 23,
                qilin_degraded_stage_d_activations: 1,
                ..Default::default()
            }),
            batch_progress: Some(crate::telemetry_bridge::BridgeBatchProgress {
                completed: 250,
                failed: 3,
                total: 2533,
                current_file: "/tmp/downloads/final.iso".to_string(),
                speed_mbps: 3.18,
                downloaded_bytes: 194 * 1_048_576,
                active_circuits: Some(16),
                bbr_bottleneck_mbps: None,
                ekf_covariance: None,
            }),
            download_progress: None,
            last_rendered: None,
            last_final_rendered: None,
            last_emitted_at: None,
            last_rss_emitted_at: None,
        };

        let rendered = render_final_progress_summary(&state);

        assert!(rendered.contains("phase=complete"));
        assert!(rendered.contains("req=87/79/8"));
        assert!(rendered.contains("subtree=3/2/0"));
        assert!(rendered.contains("dl_transport=9/4/2"));
        assert!(rendered.contains("probe_admission=7/3"));
        assert!(rendered.contains("qilin_discovery=fresh:0 stale:23 degraded:1"));
        assert!(rendered.contains("tail=winnerqualit...n5.onion/c7:8450ms/2/1"));
        assert!(rendered.contains("download=250/2533 failed=3 speed=3.18MB/s bytes=194.0MB"));
    }

    #[test]
    fn trim_summary_path_keeps_tail() {
        let rendered = trim_summary_path("/very/long/path/to/a/file/that/keeps/growing.iso", 24);
        assert!(rendered.starts_with("..."));
        assert!(rendered.ends_with("growing.iso"));
    }

    #[test]
    fn progress_summary_includes_single_file_download_when_no_batch_exists() {
        let state = CliProgressSummaryState {
            crawl_status: None,
            resource_metrics: None,
            batch_progress: None,
            download_progress: Some(crate::telemetry_bridge::BridgeDownloadProgress {
                path: "/tmp/direct/10Gb.dat".to_string(),
                bytes_downloaded: 256 * 1_048_576,
                total_bytes: Some(10 * 1024 * 1024 * 1024),
                speed_bps: 25 * 1_048_576,
                active_circuits: 32,
            }),
            last_rendered: None,
            last_final_rendered: None,
            last_emitted_at: None,
            last_rss_emitted_at: None,
        };

        let rendered = render_progress_summary(&state);

        assert!(rendered.contains("single=256.0MB/10240.0MB"));
        assert!(rendered.contains("speed=25.00MB/s"));
        assert!(rendered.contains("10Gb.dat"));
    }
}
