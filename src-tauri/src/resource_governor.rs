use crate::io_vanguard::DirectIoPolicy;
use crate::runtime_metrics::RuntimeTelemetry;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::{
    collections::HashMap,
    process::Command,
    sync::{Mutex, OnceLock},
};
use sysinfo::{DiskKind, Disks, Pid, ProcessesToUpdate, System};

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageClass {
    Hdd,
    Ssd,
    Nvme,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct ResourceGovernorProfile {
    pub cpu_cores: usize,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub storage_class: StorageClass,
    pub recommended_arti_cap: usize,
    pub recommended_quorum: usize,
    pub direct_io_policy: DirectIoPolicy,
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeGovernorSnapshot {
    pub process_cpu_percent: f64,
    pub process_memory_bytes: u64,
    pub system_memory_used_bytes: u64,
    pub system_memory_total_bytes: u64,
    pub active_workers: usize,
    pub worker_target: usize,
    pub active_circuits: usize,
    pub peak_active_circuits: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GovernorPressure {
    pub normalized_cpu_percent: f64,
    pub available_memory_ratio: f64,
    pub process_memory_ratio: f64,
    pub cpu_pressure: f64,
    pub memory_pressure: f64,
    pub io_pressure: f64,
    pub total_pressure: f64,
}

#[derive(Clone, Debug)]
pub struct RampPolicy {
    pub initial: usize,
    pub step: usize,
    pub interval_ms: u64,
}

impl Default for RampPolicy {
    fn default() -> Self {
        Self {
            initial: 1,
            step: 1,
            interval_ms: 2500,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BootstrapBudget {
    pub target_clients: usize,
    pub minimum_ready: usize,
    pub pressure: GovernorPressure,
    pub ramp_policy: RampPolicy,
}

#[derive(Clone, Debug)]
pub struct ListingBudget {
    pub worker_cap: usize,
    pub pressure: GovernorPressure,
}

#[derive(Clone, Debug)]
pub struct DownloadBudget {
    pub circuit_cap: usize,
    pub small_file_parallelism: usize,
    pub initial_active_cap: usize,
    pub tournament_cap: usize,
    pub micro_swarm_circuits: usize,
    pub pressure: GovernorPressure,
}

pub fn detect_profile(output_path: Option<&Path>) -> ResourceGovernorProfile {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);

    let mut system = System::new_all();
    system.refresh_memory();
    let total_memory_bytes = system.total_memory();
    let available_memory_bytes = system.available_memory();
    let storage_class = detect_storage_class(output_path);
    let recommended_arti_cap = recommend_arti_cap_with_storage(
        cpu_cores,
        total_memory_bytes,
        available_memory_bytes,
        storage_class,
    );
    let recommended_quorum = recommended_quorum_for_cap(recommended_arti_cap);
    let direct_io_policy = match storage_class {
        StorageClass::Hdd => DirectIoPolicy::Off,
        StorageClass::Ssd | StorageClass::Nvme | StorageClass::Unknown => DirectIoPolicy::Auto,
    };

    ResourceGovernorProfile {
        cpu_cores,
        total_memory_bytes,
        available_memory_bytes,
        storage_class,
        recommended_arti_cap,
        recommended_quorum,
        direct_io_policy,
    }
}

/// Phase 67H: System profile for GUI auto-selection of concurrency preset
#[derive(Clone, Debug, serde::Serialize)]
pub struct SystemProfile {
    pub preset: String,
    pub circuits: usize,
    pub workers: usize,
    pub cpu_cores: usize,
    pub total_ram_gb: f64,
    pub available_ram_gb: f64,
    pub storage_class: String,
    pub os: String,
}

/// Phase 67H: Returns the recommended concurrency preset based on detected hardware
pub fn recommended_concurrency_preset() -> SystemProfile {
    let profile = detect_profile(None);
    let total_ram_gb = profile.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let available_ram_gb = profile.available_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

    let (preset, circuits, workers) = if total_ram_gb <= 4.5 || profile.cpu_cores <= 2 {
        // 4GB Azure VM, ultra-low resource
        ("Conservative", 4_usize, 4_usize)
    } else if total_ram_gb <= 8.5 || profile.cpu_cores <= 4 {
        // 8GB VM or low-end hardware
        ("Balanced", 8, 8)
    } else if total_ram_gb <= 16.5 || profile.cpu_cores <= 8 {
        // Mid-range system
        ("Aggressive", 16, 16)
    } else if total_ram_gb <= 32.5 || profile.cpu_cores <= 12 {
        // High-end Mac / desktop
        ("Maximum", 32, 32)
    } else {
        // Power workstation
        ("Aerospace", 64, 64)
    };

    // Windows additional constraint (just keep it as is, max 64 now)
    let (circuits, workers) = if cfg!(target_os = "windows") {
        (circuits.min(32), workers.min(32))
    } else {
        (circuits, workers)
    };

    let os = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    };

    SystemProfile {
        preset: preset.to_string(),
        circuits,
        workers,
        cpu_cores: profile.cpu_cores,
        total_ram_gb: (total_ram_gb * 10.0).round() / 10.0,
        available_ram_gb: (available_ram_gb * 10.0).round() / 10.0,
        storage_class: storage_class_label(profile.storage_class).to_string(),
        os: os.to_string(),
    }
}

pub fn recommend_arti_cap(
    cpu_cores: usize,
    total_memory_bytes: u64,
    available_memory_bytes: u64,
) -> usize {
    recommend_arti_cap_with_storage(
        cpu_cores,
        total_memory_bytes,
        available_memory_bytes,
        StorageClass::Unknown,
    )
}

pub fn storage_class_label(class: StorageClass) -> &'static str {
    match class {
        StorageClass::Hdd => "hdd",
        StorageClass::Ssd => "ssd",
        StorageClass::Nvme => "nvme",
        StorageClass::Unknown => "unknown",
    }
}

pub fn sample_runtime_snapshot(telemetry: Option<&RuntimeTelemetry>) -> RuntimeGovernorSnapshot {
    let mut system = System::new_all();
    let pid = Pid::from(std::process::id() as usize);
    system.refresh_cpu_usage();
    system.refresh_memory();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

    if let Some(telemetry) = telemetry {
        let snapshot = telemetry.snapshot_with_system(&mut system);
        return RuntimeGovernorSnapshot {
            process_cpu_percent: snapshot.process_cpu_percent,
            process_memory_bytes: snapshot.process_memory_bytes,
            system_memory_used_bytes: snapshot.system_memory_used_bytes,
            system_memory_total_bytes: snapshot.system_memory_total_bytes,
            active_workers: snapshot.active_workers,
            worker_target: snapshot.worker_target,
            active_circuits: snapshot.active_circuits,
            peak_active_circuits: snapshot.peak_active_circuits,
        };
    }

    let (process_cpu_percent, process_memory_bytes) = if let Some(process) = system.process(pid) {
        (process.cpu_usage() as f64, process.memory())
    } else {
        (0.0, 0)
    };

    RuntimeGovernorSnapshot {
        process_cpu_percent,
        process_memory_bytes,
        system_memory_used_bytes: system.used_memory(),
        system_memory_total_bytes: system.total_memory(),
        active_workers: 0,
        worker_target: 0,
        active_circuits: 0,
        peak_active_circuits: 0,
    }
}

pub fn measure_pressure(
    profile: &ResourceGovernorProfile,
    snapshot: &RuntimeGovernorSnapshot,
) -> GovernorPressure {
    let total_memory = snapshot
        .system_memory_total_bytes
        .max(profile.total_memory_bytes)
        .max(1);
    let used_memory = snapshot.system_memory_used_bytes;
    let available_memory_ratio =
        (total_memory.saturating_sub(used_memory) as f64 / total_memory as f64).clamp(0.0, 1.0);
    let process_memory_ratio =
        (snapshot.process_memory_bytes as f64 / total_memory as f64).clamp(0.0, 1.0);
    let normalized_cpu_percent = snapshot.process_cpu_percent / profile.cpu_cores.max(1) as f64;

    let cpu_pressure = if normalized_cpu_percent <= 12.0 {
        0.0
    } else {
        ((normalized_cpu_percent - 12.0) / 28.0).clamp(0.0, 1.0)
    };

    let available_shortage = ((0.22 - available_memory_ratio) / 0.22).clamp(0.0, 1.0);
    let process_rss_pressure = ((process_memory_ratio - 0.18) / 0.18).clamp(0.0, 1.0);
    let memory_pressure = available_shortage.max(process_rss_pressure * 0.8);

    let (circuit_reference, worker_reference) = match profile.storage_class {
        StorageClass::Hdd => (8.0, 16.0),
        StorageClass::Ssd => (16.0, 40.0),
        StorageClass::Nvme => (24.0, 72.0),
        StorageClass::Unknown => (12.0, 28.0),
    };

    let io_pressure = ((snapshot.active_circuits as f64 / circuit_reference)
        .max(snapshot.worker_target as f64 / worker_reference)
        .max(snapshot.active_workers as f64 / worker_reference))
    .clamp(0.0, 1.0);

    let total_pressure =
        (cpu_pressure * 0.45 + memory_pressure * 0.35 + io_pressure * 0.20).clamp(0.0, 1.0);

    GovernorPressure {
        normalized_cpu_percent,
        available_memory_ratio,
        process_memory_ratio,
        cpu_pressure,
        memory_pressure,
        io_pressure,
        total_pressure,
    }
}

pub fn recommend_bootstrap_budget(
    requested: usize,
    output_path: Option<&Path>,
    telemetry: Option<&RuntimeTelemetry>,
) -> BootstrapBudget {
    let profile = detect_profile(output_path);
    let snapshot = sample_runtime_snapshot(telemetry);
    let pressure = measure_pressure(&profile, &snapshot);
    let hard_cap = env_cap("CRAWLI_ARTI_ACTIVE_TARGET_MAX")
        .unwrap_or(match profile.storage_class {
            StorageClass::Hdd => 8,
            StorageClass::Ssd => 16,
            StorageClass::Nvme => 24,
            StorageClass::Unknown => 12,
        })
        .clamp(1, 24);
    let base_target = requested
        .max(1)
        .min(profile.recommended_arti_cap)
        .min(hard_cap);
    let target_clients = apply_pressure_to_budget(
        base_target,
        1,
        hard_cap.min(profile.recommended_arti_cap.max(1)),
        pressure.total_pressure,
        !matches!(profile.storage_class, StorageClass::Hdd),
    );
    let minimum_ready = env_cap("CRAWLI_ARTI_MIN_READY_CLIENTS")
        .unwrap_or(recommended_quorum_for_cap(target_clients))
        .clamp(1, target_clients);

    let ramp_initial = env_cap("CRAWLI_VANGUARD_INITIAL").unwrap_or(1);
    let ramp_interval_ms = std::env::var("CRAWLI_VANGUARD_RAMP_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2500);

    let ramp_policy = RampPolicy {
        initial: ramp_initial.clamp(1, target_clients),
        step: 1,
        interval_ms: ramp_interval_ms,
    };

    BootstrapBudget {
        target_clients,
        minimum_ready,
        pressure,
        ramp_policy,
    }
}

pub fn recommend_frontier_worker_cap(
    requested_circuits: usize,
    is_onion: bool,
    reserve_for_downloads: bool,
    telemetry: Option<&RuntimeTelemetry>,
) -> usize {
    let profile = detect_profile(None);
    let snapshot = sample_runtime_snapshot(telemetry);
    let pressure = measure_pressure(&profile, &snapshot);

    let storage_cap = match profile.storage_class {
        StorageClass::Hdd => {
            if reserve_for_downloads {
                24
            } else {
                36
            }
        }
        StorageClass::Ssd => {
            if reserve_for_downloads {
                64
            } else {
                96
            }
        }
        StorageClass::Nvme => {
            if reserve_for_downloads {
                96
            } else {
                144
            }
        }
        StorageClass::Unknown => {
            if reserve_for_downloads {
                40
            } else {
                72
            }
        }
    };

    let default_cap = if is_onion {
        if reserve_for_downloads {
            72
        } else {
            120
        }
    } else {
        180
    };
    let base = requested_circuits.max(1).min(default_cap).min(storage_cap);
    apply_pressure_to_budget(
        base,
        8,
        storage_cap.min(default_cap),
        pressure.total_pressure,
        false,
    )
}

pub fn recommend_listing_budget(
    active_clients: usize,
    permit_budget: usize,
    is_onion: bool,
    reserve_for_downloads: bool,
    telemetry: Option<&RuntimeTelemetry>,
) -> ListingBudget {
    let profile = detect_profile(None);
    let snapshot = sample_runtime_snapshot(telemetry);
    let pressure = measure_pressure(&profile, &snapshot);
    listing_budget_for_profile(
        active_clients,
        permit_budget,
        is_onion,
        reserve_for_downloads,
        &profile,
        pressure,
    )
}

fn listing_budget_for_profile(
    active_clients: usize,
    permit_budget: usize,
    is_onion: bool,
    reserve_for_downloads: bool,
    profile: &ResourceGovernorProfile,
    pressure: GovernorPressure,
) -> ListingBudget {
    let limit = permit_budget.max(1);
    let base = if is_onion {
        if reserve_for_downloads {
            ((active_clients.max(1) * 2) / 3).clamp(4.min(limit), limit)
        } else {
            active_clients.max(1).clamp(6.min(limit), limit)
        }
    } else {
        (permit_budget / 2).clamp(4.min(limit), limit)
    };

    let storage_cap = match profile.storage_class {
        StorageClass::Hdd => {
            if reserve_for_downloads {
                8
            } else {
                16
            }
        }
        StorageClass::Ssd => {
            if reserve_for_downloads {
                14
            } else {
                24
            }
        }
        StorageClass::Nvme => {
            if reserve_for_downloads {
                18
            } else {
                32
            }
        }
        StorageClass::Unknown => {
            if reserve_for_downloads {
                10
            } else {
                18
            }
        }
    };

    let worker_cap = apply_pressure_to_budget(
        base.min(storage_cap).min(permit_budget.max(1)),
        4,
        storage_cap.min(permit_budget.max(1)).max(4),
        pressure.total_pressure,
        matches!(profile.storage_class, StorageClass::Nvme),
    );

    ListingBudget {
        worker_cap,
        pressure,
    }
}

pub fn recommend_download_budget(
    requested_circuits: usize,
    content_length: Option<u64>,
    is_onion: bool,
    output_path: Option<&Path>,
    telemetry: Option<&RuntimeTelemetry>,
) -> DownloadBudget {
    let profile = detect_profile(output_path);
    let snapshot = sample_runtime_snapshot(telemetry);
    let pressure = measure_pressure(&profile, &snapshot);
    download_budget_for_profile(
        requested_circuits,
        content_length,
        is_onion,
        &profile,
        pressure,
    )
}

fn download_budget_for_profile(
    requested_circuits: usize,
    content_length: Option<u64>,
    is_onion: bool,
    profile: &ResourceGovernorProfile,
    pressure: GovernorPressure,
) -> DownloadBudget {
    let download_storage_class = if is_onion && content_length.is_none() {
        match profile.storage_class {
            StorageClass::Nvme => StorageClass::Ssd,
            other => other,
        }
    } else {
        profile.storage_class
    };

    let base_cap: usize = match download_storage_class {
        StorageClass::Hdd => 8,
        StorageClass::Ssd => 16,
        StorageClass::Nvme => 24,
        StorageClass::Unknown => 12,
    };
    let onion_cap = if is_onion {
        base_cap.saturating_mul(2) // IDM-tier multiplexing stream packing
    } else {
        base_cap.saturating_mul(2)
    };
    let content_cap = match content_length.unwrap_or(0) {
        0 => requested_circuits.max(1),
        len if len < 16 * 1024 * 1024 => 2,
        len if len < 64 * 1024 * 1024 => 4,
        len if len < 256 * 1024 * 1024 => 8,
        len if len < 1024 * 1024 * 1024 => 12,
        _ => onion_cap,
    };

    let mut circuit_cap = apply_pressure_to_budget(
        requested_circuits
            .max(1)
            .min(onion_cap)
            .min(content_cap.max(1)),
        1,
        onion_cap.max(1),
        pressure.total_pressure,
        matches!(download_storage_class, StorageClass::Nvme),
    );

    // Hidden-service batch downloads benefit from NVMe-class lane sizing, but not from
    // blindly expanding the first-wave circuit spray beyond the proven stable cap.
    if is_onion && content_length.is_none() {
        let onion_batch_cap = env_cap("CRAWLI_ONION_BATCH_CIRCUIT_CAP_MAX")
            .unwrap_or(64)
            .clamp(8, 64);
        circuit_cap = circuit_cap.min(onion_batch_cap);
    }

    // Large direct clearnet artifacts saturate sooner than hidden-service files on this path.
    // Keep the default direct-file fan-out below the point where handshake churn dominates.
    if !is_onion && content_length.unwrap_or(0) >= GIB {
        let clearnet_large_cap = env_cap("CRAWLI_CLEARNET_LARGE_CIRCUIT_CAP_MAX")
            .unwrap_or(32)
            .clamp(8, 48);
        circuit_cap = circuit_cap.min(clearnet_large_cap);
    }

    let small_file_parallelism = match download_storage_class {
        StorageClass::Hdd => circuit_cap.min(8),
        StorageClass::Ssd => circuit_cap.min(32),
        StorageClass::Nvme => circuit_cap.min(64),
        StorageClass::Unknown => circuit_cap.min(16),
    }
    .max(1);

    let initial_active_cap = match download_storage_class {
        StorageClass::Hdd => circuit_cap.min(8),
        StorageClass::Ssd => circuit_cap.min(32),
        StorageClass::Nvme => circuit_cap.min(64),
        StorageClass::Unknown => circuit_cap.min(16),
    }
    .max(1);

    let tournament_cap = match download_storage_class {
        StorageClass::Hdd => (circuit_cap + (circuit_cap / 4).max(1)).min(10),
        StorageClass::Ssd => (circuit_cap + (circuit_cap / 2).max(1)).min(24),
        StorageClass::Nvme => (circuit_cap + (circuit_cap / 2).max(2)).min(36),
        StorageClass::Unknown => (circuit_cap + (circuit_cap / 3).max(1)).min(18),
    }
    .max(circuit_cap);

    DownloadBudget {
        circuit_cap,
        small_file_parallelism,
        initial_active_cap,
        tournament_cap,
        micro_swarm_circuits: apply_pressure_to_budget(
            32,
            2,
            64,
            pressure.total_pressure,
            matches!(profile.storage_class, StorageClass::Nvme),
        )
        .max(1),
        pressure,
    }
}

fn recommended_quorum_for_cap(cap: usize) -> usize {
    match cap {
        0..=2 => 1,
        3..=4 => 2,
        5..=8 => 3,
        9..=16 => 4,
        _ => 5,
    }
}

fn recommend_arti_cap_with_storage(
    cpu_cores: usize,
    total_memory_bytes: u64,
    available_memory_bytes: u64,
    storage_class: StorageClass,
) -> usize {
    let total_gib = total_memory_bytes / GIB;
    let available_gib = available_memory_bytes / GIB;

    let mut cap = match storage_class {
        StorageClass::Hdd => {
            if cpu_cores <= 4 || total_gib <= 8 || available_gib <= 2 {
                4
            } else if cpu_cores <= 8 || total_gib <= 16 || available_gib <= 4 {
                6
            } else {
                8
            }
        }
        StorageClass::Ssd => {
            if cpu_cores >= 16 && total_gib >= 32 && available_gib >= 8 {
                16
            } else if cpu_cores >= 8 && total_gib >= 16 && available_gib >= 4 {
                12
            } else if cpu_cores <= 4 || total_gib <= 8 || available_gib <= 2 {
                4
            } else {
                8
            }
        }
        StorageClass::Nvme => {
            if cpu_cores >= 24 && total_gib >= 64 && available_gib >= 16 {
                24
            } else if cpu_cores >= 16 && total_gib >= 32 && available_gib >= 8 {
                18
            } else if cpu_cores >= 8 && total_gib >= 16 && available_gib >= 4 {
                12
            } else {
                8
            }
        }
        StorageClass::Unknown => {
            if cpu_cores <= 4 || total_gib <= 8 || available_gib <= 2 {
                4
            } else if cpu_cores <= 8 || total_gib <= 16 || available_gib <= 4 {
                6
            } else if cpu_cores >= 16 && total_gib >= 32 && available_gib >= 8 {
                10
            } else {
                8
            }
        }
    };

    if cfg!(target_os = "windows") {
        cap = cap.min(match storage_class {
            StorageClass::Nvme => 16,
            StorageClass::Ssd => 12,
            StorageClass::Hdd | StorageClass::Unknown => 8,
        });
    }

    cap.clamp(2, 24)
}

fn detect_storage_class(output_path: Option<&Path>) -> StorageClass {
    #[cfg(target_os = "macos")]
    let path = output_path.unwrap_or_else(|| Path::new("/"));
    #[cfg(not(target_os = "macos"))]
    let Some(path) = output_path
    else {
        return StorageClass::Unknown;
    };

    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    let mut best_match: Option<(DiskKind, String)> = None;
    let mut best_mount: Option<std::path::PathBuf> = None;
    let mut best_len = 0usize;

    for disk in disks.list() {
        let mount = disk.mount_point();
        if canonical.starts_with(mount) {
            let mount_len = mount.as_os_str().len();
            if mount_len >= best_len {
                best_len = mount_len;
                best_match = Some((
                    disk.kind(),
                    disk.name().to_string_lossy().to_ascii_lowercase(),
                ));
                best_mount = Some(mount.to_path_buf());
            }
        }
    }

    let detected = match best_match {
        Some((DiskKind::HDD, _)) => StorageClass::Hdd,
        Some((DiskKind::SSD, name)) => {
            if name.contains("nvme") || name.contains("raid") {
                StorageClass::Nvme
            } else {
                StorageClass::Ssd
            }
        }
        Some((_, name)) if name.contains("nvme") => StorageClass::Nvme,
        _ => StorageClass::Unknown,
    };

    #[cfg(target_os = "macos")]
    if let Some(fallback) =
        macos_storage_class_fallback(best_mount.as_deref().unwrap_or_else(|| Path::new("/")))
    {
        if storage_class_rank(fallback) > storage_class_rank(detected) {
            return fallback;
        }
    }

    detected
}

fn storage_class_rank(class: StorageClass) -> u8 {
    match class {
        StorageClass::Unknown => 0,
        StorageClass::Hdd => 1,
        StorageClass::Ssd => 2,
        StorageClass::Nvme => 3,
    }
}

#[cfg(target_os = "macos")]
fn macos_storage_class_fallback(path: &Path) -> Option<StorageClass> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<StorageClass>>>> = OnceLock::new();

    let key = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(&key) {
            return *cached;
        }
    }

    let detected = macos_diskutil_storage_class(path);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, detected);
    }
    detected
}

#[cfg(target_os = "macos")]
fn macos_diskutil_storage_class(path: &Path) -> Option<StorageClass> {
    let output = Command::new("diskutil")
        .arg("info")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_diskutil_storage_class(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
fn parse_macos_diskutil_storage_class(output: &str) -> Option<StorageClass> {
    let mut solid_state: Option<bool> = None;
    let mut protocol = String::new();
    let mut media_type = String::new();

    for line in output.lines() {
        if !line.contains(':') {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let key = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let value = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        match key.as_str() {
            "solid state" => {
                solid_state = Some(matches!(value.as_str(), "yes" | "true"));
            }
            "protocol" => protocol = value,
            "media type" => media_type = value,
            _ => {}
        }
    }

    if solid_state == Some(false) {
        return Some(StorageClass::Hdd);
    }

    let nvme_like = protocol.contains("nvme")
        || protocol.contains("apple fabric")
        || media_type.contains("nvme");
    if nvme_like {
        return Some(StorageClass::Nvme);
    }

    if solid_state == Some(true) || media_type.contains("ssd") {
        return Some(StorageClass::Ssd);
    }

    None
}

fn env_cap(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn apply_pressure_to_budget(
    base: usize,
    min_allowed: usize,
    max_allowed: usize,
    total_pressure: f64,
    allow_headroom_bonus: bool,
) -> usize {
    let scale = if total_pressure >= 0.85 {
        0.50
    } else if total_pressure >= 0.70 {
        0.66
    } else if total_pressure >= 0.55 {
        0.80
    } else if total_pressure <= 0.15 && allow_headroom_bonus {
        1.15
    } else if total_pressure <= 0.30 && allow_headroom_bonus {
        1.05
    } else {
        1.0
    };

    ((base as f64) * scale)
        .round()
        .clamp(min_allowed as f64, max_allowed as f64) as usize
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::parse_macos_diskutil_storage_class;
    use super::{
        apply_pressure_to_budget, download_budget_for_profile, listing_budget_for_profile,
        measure_pressure, recommend_arti_cap, recommend_arti_cap_with_storage,
        recommend_download_budget, GovernorPressure, ResourceGovernorProfile,
        RuntimeGovernorSnapshot, StorageClass, GIB,
    };

    fn synthetic_profile(storage_class: StorageClass) -> ResourceGovernorProfile {
        ResourceGovernorProfile {
            cpu_cores: 8,
            total_memory_bytes: 16 * GIB,
            available_memory_bytes: 8 * GIB,
            storage_class,
            recommended_arti_cap: 12,
            recommended_quorum: 4,
            direct_io_policy: match storage_class {
                StorageClass::Hdd => crate::io_vanguard::DirectIoPolicy::Off,
                StorageClass::Ssd | StorageClass::Nvme | StorageClass::Unknown => {
                    crate::io_vanguard::DirectIoPolicy::Auto
                }
            },
        }
    }

    #[test]
    fn low_resource_hosts_cap_low() {
        assert_eq!(
            recommend_arti_cap(4, 8 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024),
            4
        );
    }

    #[test]
    fn mid_resource_hosts_cap_mid() {
        assert_eq!(
            recommend_arti_cap(8, 16 * 1024 * 1024 * 1024, 6 * 1024 * 1024 * 1024),
            6
        );
    }

    #[test]
    fn nvme_hosts_can_scale_to_twenty_four() {
        let cap = recommend_arti_cap_with_storage(
            24,
            64 * 1024 * 1024 * 1024,
            20 * 1024 * 1024 * 1024,
            StorageClass::Nvme,
        );
        assert_eq!(cap, 24);
    }

    #[test]
    fn pressure_model_penalizes_cpu_and_memory_contention() {
        let profile = ResourceGovernorProfile {
            cpu_cores: 8,
            total_memory_bytes: 16 * 1024 * 1024 * 1024,
            available_memory_bytes: 2 * 1024 * 1024 * 1024,
            storage_class: StorageClass::Hdd,
            recommended_arti_cap: 6,
            recommended_quorum: 3,
            direct_io_policy: crate::io_vanguard::DirectIoPolicy::Off,
        };
        let snapshot = RuntimeGovernorSnapshot {
            process_cpu_percent: 180.0,
            process_memory_bytes: 3 * 1024 * 1024 * 1024,
            system_memory_used_bytes: 15 * 1024 * 1024 * 1024,
            system_memory_total_bytes: 16 * 1024 * 1024 * 1024,
            active_workers: 12,
            worker_target: 16,
            active_circuits: 8,
            peak_active_circuits: 8,
        };

        let pressure = measure_pressure(&profile, &snapshot);
        assert!(pressure.total_pressure > 0.6);
        assert!(pressure.memory_pressure > 0.5);
        assert!(pressure.cpu_pressure > 0.1);
    }

    #[test]
    fn pressure_can_reduce_budget() {
        let reduced = apply_pressure_to_budget(12, 1, 24, 0.9, true);
        assert!(reduced <= 6);
    }

    #[test]
    fn listing_budget_stays_conservative_on_hdd() {
        let budget = listing_budget_for_profile(
            12,
            64,
            true,
            true,
            &synthetic_profile(StorageClass::Hdd),
            GovernorPressure::default(),
        );
        assert!(budget.worker_cap <= 8);
    }

    #[test]
    fn download_budget_caps_small_file_parallelism() {
        let budget = download_budget_for_profile(
            24,
            Some(32 * 1024 * 1024),
            true,
            &synthetic_profile(StorageClass::Hdd),
            GovernorPressure::default(),
        );
        assert!(budget.circuit_cap <= 4);
        assert!(budget.small_file_parallelism <= budget.circuit_cap);
    }

    #[test]
    fn onion_batch_budget_keeps_first_wave_capped() {
        let budget = recommend_download_budget(120, None, true, None, None);
        assert!(budget.circuit_cap <= 64);
    }

    #[test]
    fn clearnet_large_download_budget_caps_nvme_fanout() {
        let budget = download_budget_for_profile(
            120,
            Some(10 * GIB),
            false,
            &synthetic_profile(StorageClass::Nvme),
            GovernorPressure::default(),
        );
        assert!(budget.circuit_cap <= 32);
        assert!(budget.initial_active_cap <= budget.circuit_cap);
    }

    #[test]
    fn governor_pressure_defaults_are_stable() {
        let pressure = GovernorPressure::default();
        assert_eq!(pressure.total_pressure, 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_diskutil_parser_detects_nvme_ssd() {
        let sample = r#"
   Media Type:                Generic
   Protocol:                  Apple Fabric
   Solid State:               Yes
"#;
        assert_eq!(
            parse_macos_diskutil_storage_class(sample),
            Some(StorageClass::Nvme)
        );
    }
}
