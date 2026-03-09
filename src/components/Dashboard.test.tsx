import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { Dashboard } from './Dashboard';

describe('Dashboard', () => {
    const defaultProps = {
        isCrawling: false,
        torStatus: { state: 'idle' },
        activeAdapter: 'Test Adapter',
        crawlStatus: {
            phase: 'IDLE',
            progressPercent: 0,
            visitedNodes: 0,
            processedNodes: 0,
            queuedNodes: 0,
            activeWorkers: 0,
            workerTarget: 4,
            etaSeconds: null,
            estimation: 'Idle'
        },
        downloadBatchStatus: {
            totalFiles: 0,
            completedFiles: 0,
            failedFiles: 0,
            totalBytesHint: 0,
            unknownSizeFiles: 0,
            currentFile: '',
            speedMbps: 0,
            smoothedSpeedMbps: 0,
            downloadedBytes: 0,
            activeCircuits: 0,
            peakActiveCircuits: 0,
            peakBandwidthMbps: 0,
            diskWriteMbps: 0,
            peakDiskWriteMbps: 0,
            etaConfidence: 0,
            outputDir: '',
            bbrBottleneckMbps: 0,
            ekfCovariance: 0,
            startedAt: null,
            etaSeconds: null
        },
        logs: [],
        vfsCount: 0,
        vfsRefreshTrigger: 0,
        downloadProgress: {},
        elapsed: 0,
        downloadElapsed: 0,
        resourceMetrics: {
            processCpuPercent: 0,
            processMemoryBytes: 0,
            processThreads: 0,
            systemMemoryUsedBytes: 0,
            systemMemoryTotalBytes: 0,
            systemMemoryPercent: 0,
            activeWorkers: 0,
            workerTarget: 0,
            activeCircuits: 0,
            peakActiveCircuits: 0,
            currentNodeHost: null,
            nodeFailovers: 0,
            throttleCount: 0,
            timeoutCount: 0,
            uptimeSeconds: 0,
            consensusWeight: 0
        },
        crawlRunStatus: null,
        downloadResumePlan: null
    };

    it('renders correctly', () => {
        const { container } = render(<Dashboard {...defaultProps} />);
        expect(container).toBeInTheDocument();
        expect(screen.getByText('OPERATION PHASE')).toBeInTheDocument();
        expect(screen.getByText('TOR SWARM')).toBeInTheDocument();
        expect(screen.getByText('Test Adapter')).toBeInTheDocument();
    });
});
