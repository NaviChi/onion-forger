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

    it('renders correctly in default state', () => {
        const { container } = render(<Dashboard {...defaultProps} />);
        expect(container).toBeInTheDocument();
        expect(screen.getByText('OPERATION PHASE')).toBeInTheDocument();
        expect(screen.getByText('TOR SWARM')).toBeInTheDocument();
        expect(screen.getByText('Test Adapter')).toBeInTheDocument();
    });

    it('handles downloadProgress properly', () => {
        const props = {
            ...defaultProps,
            downloadProgress: {
                'file1': { path: 'file1', bytes_downloaded: 100, speed_bps: 50 },
                'file2': { bytes_downloaded: 200, speed_bps: 100 }
            }
        };
        render(<Dashboard {...props} />);
        expect(screen.getByText('OPERATION PHASE')).toBeInTheDocument();
    });

    it('calculates proper phase when crawling (tor bootstrapping)', () => {
        const props = {
            ...defaultProps,
            isCrawling: true,
            torStatus: { state: 'bootstrapping' },
            activeAdapter: 'Unidentified'
        };
        render(<Dashboard {...props} />);
        expect(screen.getByText('BOOTSTRAPPING TOR NODE')).toBeInTheDocument();
        expect(screen.getByText('Handshake in progress...')).toBeInTheDocument();
    });

    it('calculates proper phase when crawling (tor ready, active adapter)', () => {
        const props = {
            ...defaultProps,
            isCrawling: true,
            torStatus: { state: 'ready' },
            activeAdapter: 'Test Adapter'
        };
        render(<Dashboard {...props} />);
        expect(screen.getByText('SCANNING / FILE LISTING')).toBeInTheDocument();
        expect(screen.getByText('Encrypted Swarm (Active)')).toBeInTheDocument();
    });

    it('calculates proper phase when downloading (Auto-Mirror log)', () => {
        const props = {
            ...defaultProps,
            isCrawling: true,
            logs: ['Auto-Mirror engaged for target', 'Some other log'],
            downloadBatchStatus: { ...defaultProps.downloadBatchStatus, totalFiles: 1 }
        };
        render(<Dashboard {...props} />);
        expect(screen.getByText('SCAFFOLDING (DOWNLOADING)')).toBeInTheDocument();
    });

    it('calculates proper phase when completed (Finish signaled log)', () => {
        const props = {
            ...defaultProps,
            isCrawling: true,
            logs: ['Finish signaled']
        };
        render(<Dashboard {...props} />);
        expect(screen.getByText('OPERATION PHASE')).toBeInTheDocument();
        // Since it checks if 'COMPLETE' is displayed
        expect(screen.getByText('Cooldown')).toBeInTheDocument();
    });
});
