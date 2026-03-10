import { useState, useEffect, useRef, useCallback } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import { Terminal, HardDrive, RefreshCcw, Layers } from "lucide-react";
import "./HexViewer.css";

interface HexViewerProps {
    url: string;
    isOpen: boolean;
    onClose: () => void;
}

const BLOCK_SIZE = 4096;
const BYTES_PER_ROW = 16;
const ROWS_PER_BLOCK = BLOCK_SIZE / BYTES_PER_ROW; // 256 rows

export function HexViewer({ url, isOpen, onClose }: HexViewerProps) {
    const [blocks, setBlocks] = useState<Record<number, Uint8Array>>({});
    const [error, setError] = useState<string | null>(null);
    const [activeLba, setActiveLba] = useState<number>(0);
    const parentRef = useRef<HTMLDivElement>(null);
    const [isDumbMode, setIsDumbMode] = useState(false);

    // Load block safely avoiding multiple duplicate fetches
    const pendingFetches = useRef<Set<number>>(new Set());

    const fetchBlock = useCallback(async (lba: number) => {
        if (blocks[lba] || pendingFetches.current.has(lba)) return;
        pendingFetches.current.add(lba);
        try {
            // Direct IPC to Rust Zero-Copy Scraper
            const buf: number[] = await invoke("fetch_network_disk_block_cmd", {
                url, lba, blockSize: BLOCK_SIZE
            });
            setBlocks(prev => ({ ...prev, [lba]: new Uint8Array(buf) }));
        } catch (err: any) {
            setError(err?.toString() || "Failed to fetch disk block");
        } finally {
            pendingFetches.current.delete(lba);
        }
    }, [url, blocks]);

    // Infinite total rows - simulate a 10GB drive (or pick an arbitrary high number for virtualization)
    // Let's pretend the disk has 1,000,000 blocks
    const MAX_BLOCKS = 1_000_000;
    const totalRows = MAX_BLOCKS * ROWS_PER_BLOCK;

    const rowVirtualizer = useVirtualizer({
        count: totalRows,
        getScrollElement: () => parentRef.current,
        estimateSize: () => 24, // Row height in pixels (matches CSS)
        overscan: 20,
    });

    const virtualItems = rowVirtualizer.getVirtualItems();

    useEffect(() => {
        if (!isOpen) return;
        // Check which blocks are visible and fetch them
        if (virtualItems.length > 0) {
            const firstRow = virtualItems[0].index;
            const lastRow = virtualItems[virtualItems.length - 1].index;

            const firstBlock = Math.floor(firstRow / ROWS_PER_BLOCK);
            const lastBlock = Math.floor(lastRow / ROWS_PER_BLOCK);

            setActiveLba(firstBlock);

            for (let b = firstBlock; b <= lastBlock; b++) {
                fetchBlock(b);
            }
        }
    }, [isOpen, virtualItems, fetchBlock]);

    if (!isOpen) return null;

    return (
        <div className="hex-viewer-overlay">
            <div className="hex-viewer-modal">
                <div className="hex-viewer-header">
                    <div className="hex-viewer-title">
                        <HardDrive size={18} className="text-blue-400" />
                        <h3>Native Network Disk Hex Viewer</h3>
                        <span className="hex-viewer-badge">ZERO-COPY DOD</span>
                    </div>
                    <div className="hex-viewer-actions">
                        <button
                            className={`dumb-mode-toggle ${isDumbMode ? "active" : ""}`}
                            onClick={() => setIsDumbMode(!isDumbMode)}
                            title="Toggle CRAWLI_DUMB_MODE Fast-Path Logic for synthentic benchmark saturation"
                        >
                            <RefreshCcw size={14} /> Turbo Bypass
                        </button>
                        <button className="close-button" onClick={onClose}>&times;</button>
                    </div>
                </div>

                <div className="hex-metadata-bar">
                    <div className="metadata-item">
                        <Layers size={14} />
                        <span>Target: <span className="mono">{url || "NO_TARGET"}</span></span>
                    </div>
                    <div className="metadata-item">
                        <Terminal size={14} />
                        <span>Active LBA: <span className="mono">0x{activeLba.toString(16).padStart(8, '0').toUpperCase()}</span></span>
                    </div>
                    <div className="metadata-item">
                        <span>Align: <span className="mono text-green-400">#[repr(align(64))]</span></span>
                    </div>
                </div>

                {error && <div className="hex-error-bar">{error}</div>}

                <div className="hex-grid-container">
                    <div className="hex-grid-header">
                        <div className="hex-col-addr">OFFSET</div>
                        <div className="hex-col-bytes">
                            {[...Array(16)].map((_, i) => (
                                <span key={i}>{i.toString(16).toUpperCase().padStart(2, '0')}</span>
                            ))}
                        </div>
                        <div className="hex-col-ascii">DECODED ASCII</div>
                    </div>

                    <div ref={parentRef} className="hex-scroll-area">
                        <div style={{ height: `${rowVirtualizer.getTotalSize()}px`, width: '100%', position: 'relative' }}>
                            {virtualItems.map((virtualRow) => {
                                const globalRowIndex = virtualRow.index;
                                const blockIndex = Math.floor(globalRowIndex / ROWS_PER_BLOCK);
                                const localRowIndex = globalRowIndex % ROWS_PER_BLOCK;

                                const blockData = blocks[blockIndex];
                                const byteOffset = localRowIndex * BYTES_PER_ROW;
                                const absoluteOffset = blockIndex * BLOCK_SIZE + byteOffset;

                                let hexBytes = Array(16).fill("..");
                                let asciiChars = Array(16).fill(".");

                                if (blockData) {
                                    for (let i = 0; i < BYTES_PER_ROW; i++) {
                                        const absIndex = byteOffset + i;
                                        if (absIndex < blockData.length) {
                                            const byte = blockData[absIndex];
                                            hexBytes[i] = byte.toString(16).padStart(2, '0').toUpperCase();
                                            asciiChars[i] = (byte >= 32 && byte <= 126) ? String.fromCharCode(byte) : ".";
                                        }
                                    }
                                } else {
                                    // Rendering skeleton for un-fetched rows
                                }

                                return (
                                    <div
                                        key={virtualRow.key}
                                        className={`hex-row ${localRowIndex === 0 ? "lba-boundary" : ""}`}
                                        style={{
                                            position: 'absolute',
                                            top: 0,
                                            left: 0,
                                            width: '100%',
                                            height: `${virtualRow.size}px`,
                                            transform: `translateY(${virtualRow.start}px)`,
                                        }}
                                    >
                                        <div className="hex-col-addr">
                                            0x{absoluteOffset.toString(16).padStart(8, '0').toUpperCase()}
                                        </div>
                                        <div className="hex-col-bytes">
                                            {hexBytes.map((byte, i) => (
                                                <span key={i} className={byte === "00" ? "byte-zero" : (byte !== ".." && blockData ? "byte-val" : "byte-empty")}>
                                                    {byte}
                                                </span>
                                            ))}
                                        </div>
                                        <div className="hex-col-ascii">
                                            {asciiChars.map((char, i) => (
                                                <span key={i} className={char === "." ? "ascii-dot" : "ascii-char"}>
                                                    {char}
                                                </span>
                                            ))}
                                        </div>
                                    </div>
                                );
                            })}
                        </div>
                    </div>
                </div>

            </div>
        </div>
    );
}
