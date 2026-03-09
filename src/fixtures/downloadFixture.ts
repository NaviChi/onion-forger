export interface FixtureDownloadTrigger {
    url: string;
    path: string;
}

/**
 * Validates if the browser is currently running the isolated E2E download fixture harness.
 */
export function isDownloadFixtureMode(): boolean {
    if (typeof window === "undefined") return false;
    const params = new URLSearchParams(window.location.search);
    return params.get("fixture") === "download";
}

/**
 * Triggers a simulated cascade of __TAURI_IPC__ download progress events
 * against the global `window.dispatchEvent` router, allowing React to bind
 * mock telemetry without a native WebKit/Chromium container.
 */
export function simulateDownloadProgress(targets: FixtureDownloadTrigger[]) {
    if (typeof window === "undefined") return;

    const totalFiles = targets.length;
    const totalBytesHint = targets.length * 1048576 * 50; // 50MB per file

    // 1. Fire Batch Started
    window.dispatchEvent(
        new CustomEvent("tauri://download_batch_started", {
            detail: {
                payload: {
                    totalFiles,
                    totalBytesHint,
                    unknownSizeFiles: 0,
                    outputDir: "/mock/OnionForger_Downloads"
                }
            }
        })
    );

    let elapsed = 0;
    let downloadedBytes = 0;
    let completedFiles = 0;

    const interval = setInterval(() => {
        elapsed += 1;
        const isFinished = completedFiles >= totalFiles;

        if (isFinished) {
            clearInterval(interval);
            return;
        }

        const currentTarget = targets[completedFiles];
        const chunkSequence = 1048576 * 15; // 15MB/s
        downloadedBytes += chunkSequence;

        const fileDownloaded = (elapsed * chunkSequence) % (1048576 * 50);
        const speedMbps = 15;
        const bbrBottleneckMbps = 18;
        const ekfCovariance = 0.05;

        if (fileDownloaded === 0 && elapsed > 1) {
            // File finished
            window.dispatchEvent(
                new CustomEvent("tauri://complete", {
                    detail: {
                        payload: {
                            url: currentTarget.url,
                            path: currentTarget.path,
                            hash: "mock-sha256-hash",
                            time_taken_secs: 3
                        }
                    }
                })
            );
            completedFiles += 1;
        }

        // 2. Fire File Progress
        if (completedFiles < totalFiles) {
            const activeTarget = targets[completedFiles];
            window.dispatchEvent(
                new CustomEvent("tauri://telemetry_bridge_update", {
                    detail: {
                        payload: {
                            downloadProgress: [{
                                path: activeTarget.path,
                                bytes_downloaded: fileDownloaded,
                                total_bytes: 1048576 * 50,
                                speed_bps: speedMbps * 1048576,
                                active_circuits: 6
                            }],
                            batchProgress: {
                                completed: completedFiles,
                                failed: 0,
                                total: totalFiles,
                                currentFile: activeTarget.path,
                                speedMbps,
                                downloadedBytes,
                                activeCircuits: 12,
                                bbrBottleneckMbps,
                                ekfCovariance
                            }
                        }
                    }
                })
            );
        }
    }, 1000);
}
