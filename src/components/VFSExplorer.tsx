import React, { useState, useEffect, useMemo, useCallback } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, ChevronDown, Folder, FileIcon, DownloadCloud } from 'lucide-react';
import { VibeLoader } from './VibeLoader';
import { VFS_FIXTURE_ENTRIES, fixtureParentPath, isVfsFixtureMode, normalizeVfsPath } from "../fixtures/vfsFixture";

export type EntryType = 'File' | 'Folder';

export interface FileEntry {
    path: string;
    size_bytes: number | null;
    entry_type: EntryType;
    raw_url: string;
}

export interface DownloadProgressEvent {
    path: string;
    bytes_downloaded: number;
    total_bytes: number | null;
    speed_bps: number;
    active_circuits?: number;
}

export interface VFSTreeNode {
    id: string; // The full path
    name: string;
    type: EntryType;
    size: number | null;
    raw_url: string;
    depth: number;
    isExpanded: boolean;
    childrenLoaded: boolean;
    isLoading: boolean;
    childrenPaths: string[]; // List of child IDs
}

function formatBytes(bytes: number | null) {
    if (bytes === null) return '--';
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

export function VFSExplorer({
    triggerRefresh,
    onDownload,
    onSelectionChange,
    downloadProgress
}: {
    triggerRefresh: number, // Incremented when new nodes arrive
    onDownload: (url: string, path: string) => void,
    onSelectionChange?: (selected: FileEntry[]) => void,
    downloadProgress?: Record<string, DownloadProgressEvent>
}) {
    const [nodes, setNodes] = useState<Record<string, VFSTreeNode>>({});
    const [rootPaths, setRootPaths] = useState<string[]>([]);
    const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
    const [isInitialLoading, setIsInitialLoading] = useState(true);
    const isTauriRuntime = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
    const isFixtureMode = !isTauriRuntime && isVfsFixtureMode();

    const loadChildren = async (parentPath: string, depth: number) => {
        if (isFixtureMode) {
            const parent = normalizeVfsPath(parentPath);
            const children = VFS_FIXTURE_ENTRIES
                .filter((entry) => fixtureParentPath(entry.path) === parent)
                .map((entry) => ({ ...entry, path: normalizeVfsPath(entry.path) }))
                .sort((a, b) => {
                    if (a.entry_type !== b.entry_type) {
                        return a.entry_type === "Folder" ? -1 : 1;
                    }
                    return a.path.localeCompare(b.path);
                });

            setNodes((prev) => {
                const next = { ...prev };
                const childIds: string[] = [];

                children.forEach((child) => {
                    const name = child.path.split('/').pop() || child.path;
                    if (!next[child.path]) {
                        next[child.path] = {
                            id: child.path,
                            name,
                            type: child.entry_type,
                            size: child.size_bytes,
                            raw_url: child.raw_url,
                            depth,
                            isExpanded: false,
                            childrenLoaded: false,
                            isLoading: false,
                            childrenPaths: []
                        };
                    }
                    childIds.push(child.path);
                });

                if (parentPath === '') {
                    setRootPaths(childIds);
                } else if (next[parentPath]) {
                    next[parentPath] = {
                        ...next[parentPath],
                        isLoading: false,
                        childrenLoaded: true,
                        childrenPaths: childIds
                    };
                }
                return next;
            });
            setIsInitialLoading(false);
            return;
        }

        if (!isTauriRuntime) {
            if (parentPath === "") {
                setRootPaths([]);
                setIsInitialLoading(false);
            }
            return;
        }

        try {
            if (parentPath !== '') {
                setNodes(prev => ({
                    ...prev,
                    [parentPath]: { ...prev[parentPath], isLoading: true }
                }));
            }

            const children: FileEntry[] = await invoke("get_vfs_children", { parentPath });

            setNodes(prev => {
                const next = { ...prev };
                const childIds: string[] = [];

                children.forEach(child => {
                    const name = child.path.split('/').pop() || child.path;
                    // Only insert if it doesn't exist, to preserve expansion state
                    if (!next[child.path]) {
                        next[child.path] = {
                            id: child.path,
                            name,
                            type: child.entry_type,
                            size: child.size_bytes,
                            raw_url: child.raw_url,
                            depth: depth,
                            isExpanded: false,
                            childrenLoaded: false,
                            isLoading: false,
                            childrenPaths: []
                        };
                    }
                    childIds.push(child.path);
                });

                if (parentPath === '') {
                    // We are fetching root
                    setRootPaths(childIds);
                } else if (next[parentPath]) {
                    next[parentPath] = {
                        ...next[parentPath],
                        isLoading: false,
                        childrenLoaded: true,
                        childrenPaths: childIds
                    };
                }

                return next;
            });
        } catch (e) {
            console.error("Failed to fetch VFS children:", e);
        } finally {
            if (parentPath === '') setIsInitialLoading(false);
        }
    };

    // Initial Load & Refresh
    useEffect(() => {
        loadChildren('', 0);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [triggerRefresh, isTauriRuntime, isFixtureMode]);

    const toggleNode = useCallback(async (id: string) => {
        const node = nodes[id];
        if (!node || node.type !== 'Folder') return;

        if (node.isExpanded) {
            // Collapse
            setNodes(prev => ({
                ...prev,
                [id]: { ...prev[id], isExpanded: false }
            }));
        } else {
            // Expand
            // If children not loaded, fetch them
            if (!node.childrenLoaded) {
                await loadChildren(id, node.depth + 1);
            }
            // Once loaded (or if already loaded), mark expanded
            setNodes(prev => ({
                ...prev,
                [id]: { ...prev[id], isExpanded: true }
            }));
        }
    }, [nodes]);

    const toggleSelection = useCallback((id: string) => {
        const newSet = new Set(selectedIds);
        const isSelected = newSet.has(id);

        // Helper to recursively collect all ALREADY LOADED children
        const collectChildren = (currentId: string, out: string[]) => {
            out.push(currentId);
            const nd = nodes[currentId];
            if (nd && nd.childrenPaths) {
                nd.childrenPaths.forEach(c => collectChildren(c, out));
            }
        };

        const idsToToggle: string[] = [];
        collectChildren(id, idsToToggle);

        idsToToggle.forEach(cid => {
            if (isSelected) newSet.delete(cid);
            else newSet.add(cid);
        });

        setSelectedIds(newSet);

        if (onSelectionChange) {
            const selectedEntries: FileEntry[] = [];
            newSet.forEach(selId => {
                const n = nodes[selId];
                if (n) {
                    selectedEntries.push({
                        path: n.id,
                        size_bytes: n.size,
                        entry_type: n.type,
                        raw_url: n.raw_url
                    });
                }
            });
            onSelectionChange(selectedEntries);
        }
    }, [nodes, selectedIds, onSelectionChange]);

    // Compute flattened visible paths for the virtualizer based on expansion state
    const visiblePaths = useMemo(() => {
        const result: string[] = [];

        const traverse = (currentId: string) => {
            const node = nodes[currentId];
            if (!node) return;

            result.push(currentId);

            if (node.isExpanded) {
                node.childrenPaths.forEach(traverse);
            }
        };

        rootPaths.forEach(traverse);
        return result;
    }, [nodes, rootPaths]);

    // Virtualizer
    const parentRef = React.useRef<HTMLDivElement>(null);

    const virtualizer = useVirtualizer({
        count: visiblePaths.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => 36,
        overscan: 20,
    });

    if (isInitialLoading) {
        return (
            <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-muted)', gap: '8px' }}>
                <VibeLoader size={18} /> Indexing Database Streams...
            </div>
        );
    }

    if (rootPaths.length === 0) {
        return (
            <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-muted)' }}>
                No files detected in virtual file system.
            </div>
        );
    }

    return (
        <div ref={parentRef} className="vfs-container" style={{ height: '100%', overflow: 'auto' }}>
            <div
                style={{
                    height: `${virtualizer.getTotalSize()}px`,
                    width: '100%',
                    position: 'relative',
                }}
            >
                {virtualizer.getVirtualItems().map((virtualRow) => {
                    const id = visiblePaths[virtualRow.index];
                    const node = nodes[id];
                    if (!node) return null;

                    return (
                        <div
                            key={node.id}
                            className={`vfs-row ${selectedIds.has(node.id) ? 'selected' : ''}`}
                            data-testid={`vfs-row-${encodeURIComponent(node.id)}`}
                            style={{
                                position: 'absolute',
                                top: 0,
                                left: 0,
                                width: '100%',
                                height: `${virtualRow.size}px`,
                                transform: `translateY(${virtualRow.start}px)`,
                                paddingLeft: `${node.depth * 24 + 12}px`,
                            }}
                        >

                            <div style={{ paddingRight: '12px', display: 'flex', alignItems: 'center' }}>
                                <input
                                    type="checkbox"
                                    checked={selectedIds.has(node.id)}
                                    onChange={() => toggleSelection(node.id)}
                                    data-testid={`vfs-select-${encodeURIComponent(node.id)}`}
                                    style={{ accentColor: 'var(--accent-primary)', width: '14px', height: '14px', cursor: 'pointer' }}
                                />
                            </div>

                            <div
                                className="vfs-toggle"
                                onClick={() => node.type === 'Folder' && toggleNode(node.id)}
                                data-testid={`vfs-toggle-${encodeURIComponent(node.id)}`}
                                style={{ visibility: node.type === 'Folder' ? 'visible' : 'hidden', display: 'flex', alignItems: 'center', justifyContent: 'center' }}
                            >
                                {node.isLoading ? (
                                    <VibeLoader size={12} variant="accent" />
                                ) : node.isExpanded ? (
                                    <ChevronDown size={14} />
                                ) : (
                                    <ChevronRight size={14} />
                                )}
                            </div>

                            <div className="vfs-icon">
                                {node.type === 'Folder' ? (
                                    <Folder size={16} fill="var(--accent-primary)" color="var(--accent-primary)" />
                                ) : (
                                    <FileIcon size={16} color="var(--text-muted)" />
                                )}
                            </div>

                            <span className="vfs-name">{node.name}</span>

                            {node.type === 'File' && (
                                <>
                                    {downloadProgress && downloadProgress[node.id] ? (
                                        (() => {
                                            const dl = downloadProgress[node.id];
                                            const sizeKnown = dl.total_bytes !== null && dl.total_bytes > 0;
                                            const activePercent = sizeKnown
                                                ? Math.min(100, Math.floor((dl.bytes_downloaded / dl.total_bytes!) * 100))
                                                : 100;

                                            const isDone = (sizeKnown && dl.bytes_downloaded >= dl.total_bytes!) || (dl.speed_bps === 0 && dl.bytes_downloaded > 0);

                                            return (
                                                <div style={{ flex: 1, position: 'relative', height: '100%', display: 'flex', alignItems: 'center', marginLeft: '12px', paddingRight: '24px' }}>
                                                    <div style={{
                                                        position: 'absolute',
                                                        left: 0,
                                                        top: '10%',
                                                        height: '80%',
                                                        width: `${activePercent}%`,
                                                        backgroundColor: isDone ? 'rgba(16, 185, 129, 0.2)' : 'rgba(139, 92, 246, 0.2)',
                                                        borderLeft: isDone ? '2px solid rgb(16, 185, 129)' : '2px solid rgb(139, 92, 246)',
                                                        borderRadius: '2px',
                                                        transition: 'width 0.3s ease, background-color 0.3s ease'
                                                    }} />
                                                    <div style={{ position: 'relative', zIndex: 1, display: 'flex', width: '100%', justifyContent: 'space-between', paddingLeft: '8px', fontSize: '0.75rem', fontFamily: 'JetBrains Mono', color: isDone ? '#10B981' : 'var(--text-muted)' }}>
                                                        <span style={{ display: 'flex', alignItems: 'center' }}>
                                                            {isDone ? 'COMPLETED' : `${formatBytes(dl.speed_bps)}/s`}
                                                            {!isDone && dl.active_circuits ? <span style={{ color: "var(--accent-primary)", marginLeft: "8px" }}>[{dl.active_circuits} Nodes]</span> : null}
                                                        </span>
                                                        <span>{activePercent}%</span>
                                                    </div>
                                                </div>
                                            );
                                        })()
                                    ) : (
                                        <div style={{ flex: 1, display: 'flex', justifyContent: 'flex-end', gap: '16px', alignItems: 'center', paddingRight: '12px' }}>
                                            <span className="vfs-size" style={{ width: '80px', textAlign: 'right' }}>{formatBytes(node.size)}</span>
                                            <button
                                                className="vfs-download-btn"
                                                onClick={() => onDownload(node.raw_url, node.id)}
                                                data-testid={`vfs-download-${encodeURIComponent(node.id)}`}
                                                style={{ display: 'flex', alignItems: 'center', gap: '4px', opacity: 0, transition: 'opacity 0.2s', padding: '4px 8px' }}
                                            >
                                                <DownloadCloud size={14} /> DL
                                            </button>
                                        </div>
                                    )}
                                </>
                            )}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
