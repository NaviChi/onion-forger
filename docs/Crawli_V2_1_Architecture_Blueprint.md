# Crawli V2.1 Architecture Blueprint

## Executive Summary
Provides a structural overview of the V2.1 optimizations delivered for the Crawli decentralized network ingestion engine, primarily focusing on extreme HDD constraint handling, asynchronous downloading pooling, and adapter plugin extensibility.

## Key Subsystems

### 1. Dual-Swarm Download Architecture (`aria_downloader.rs`)
To prevent Tor circuit congestion and small-file protocol overhead, the download engine has been bifurcated.
- **Large Files (>5MB):** Processed via the traditional `DownloadBudget` pipeline featuring `Stealing Queue` and `BBR Pacing`.
- **Micro Files (<5MB):** Dispatched immediately to a standalone background `JoinSet` bounded by `micro_swarm_circuits`. This prevents hundreds of tiny files from stalling the main BBR logic pipeline and maximizes multiplexed HTTP/2 streams across Tor guard relays.

### 2. Live VFS Visualization (`VfsTreeView.tsx`)
A recursive, dynamic UI tree view implemented over `@tanstack/react-virtual` logic.
- Displays `EntryType::Folder` and `EntryType::File` payloads.
- **Heatmap Integration:** The React layer requests `get_subtree_heatmap` dynamically, painting deep-branch failures red and clean-extractions green directly onto the DOM using custom CSS opacities.
- **OS Extensibility:** Incorporates `open_folder_os` Tauri commands for immediate zero-friction target inspection.

### 3. Binary Telemetry Constraints
Given the 4GB HDD VM restriction, writing raw Protobuf telemetry frames constantly is a heavy IO penalty.
- Introduced `TELEMETRY_ENABLED` `AtomicBool`, managed via the `set_telemetry_enabled` Tauri bridge.
- Defaults to `false`, allowing standard debug outputs without thrashing the limited HDD sectors.

### 4. Adapter Execution Engine (`adapter_pipeline_trait.rs`)
To support hot-swapping parsers for changing leak site schemas without recompiling the monolithic crawler:
- `CrawlAdapter` trait acts as the universal sink.
- Included the `RhaiScriptAdapter` for immediate usage. By utilizing the `rhai` Rust crate, we avoid the 40MB minimum binary overhead of `wasmtime` while still keeping the script sandbox capable of safely executing parsed string variables into `Vec<FileEntry>`.

## OS Level Prevention Rules Updated
1. **Never overload `JoinSet` polling.** When dispatching hundreds of concurrent tasks, manually enforce limit bounds (e.g. `micro_parallelism`) to ensure `tokio` thread starvation does not occur on 2-core VMs.
2. **HDD Write Synchronization.** Heavy payload processing (such as the heatmap calculation or protobuf stream mapping) runs lazily out-of-band to safeguard synchronous filesystem `.ariaforge` commit cycles.
