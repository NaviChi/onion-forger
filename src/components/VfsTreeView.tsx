import React, { useState, useEffect, useMemo, useCallback } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { ChevronRight, ChevronDown, Folder, FileIcon, ExternalLink, FileText, Code2 } from 'lucide-react';
import { VibeLoader } from './VibeLoader';
import { VFS_FIXTURE_ENTRIES, isVfsFixtureMode } from "../fixtures/vfsFixture";
import { invokeCommand, isTauriRuntime as getIsTauriRuntime } from "../platform/tauriClient";
import { isDirectTreeChild, normalizeTreePath } from "../utils/vfsPath";

export type EntryType = 'File' | 'Folder';

export interface FileEntry {
    path: string;
    size_bytes: number | null;
    entry_type: EntryType;
    raw_url: string;
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

function TreeRow({
    node,
    heatColor,
    heatScore,
    onToggleNode,
    style,
}: {
    node: VFSTreeNode;
    heatColor: string;
    heatScore: number;
    onToggleNode: (id: string) => void;
    style?: React.CSSProperties;
}) {
    return (
        <div
            className="vfs-row"
            data-testid={`vfs-row-${encodeURIComponent(node.id)}`}
            style={{
                backgroundColor: heatColor,
                borderLeft: heatScore > 3 ? `2px solid var(--accent-alert)` : 'none',
                ...style,
            }}
        >
            <div
                className="vfs-toggle"
                onClick={() => node.type === 'Folder' && onToggleNode(node.id)}
                style={{ visibility: node.type === 'Folder' ? 'visible' : 'hidden', display: 'flex' }}
            >
                {node.isLoading ? <VibeLoader size={12} variant="accent" /> : (node.isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />)}
            </div>
            <div className="vfs-icon">
                {node.type === 'Folder' ? <Folder size={14} fill="var(--accent-primary)" color="var(--accent-primary)" /> : <FileIcon size={14} color="var(--text-muted)" />}
            </div>
            <span className="vfs-name" style={{ fontSize: '13px' }}>{node.name}</span>
            {node.type === 'File' && (
                <div style={{ flex: 1, display: 'flex', justifyContent: 'flex-end', paddingRight: '16px' }}>
                    <span className="vfs-size">{formatBytes(node.size)}</span>
                </div>
            )}
        </div>
    );
}

function computeHeatState(id: string, nodeType: EntryType, heatmap: Record<string, any>) {
    let heatColor = "transparent";
    let heatScore = 0;
    if (nodeType === 'Folder') {
        const match = Object.entries(heatmap).find(([k]) => id.startsWith(k));
        if (match) {
            heatScore = match[1].heat_score || 0;
            if (heatScore > 10) heatColor = "rgba(255, 0, 85, 0.2)";
            else if (heatScore > 3) heatColor = "rgba(255, 165, 0, 0.15)";
        }
    }
    return { heatColor, heatScore };
}

function SimpleTreeList({
    visiblePaths,
    nodes,
    heatmap,
    toggleNode,
}: {
    visiblePaths: string[];
    nodes: Record<string, VFSTreeNode>;
    heatmap: Record<string, any>;
    toggleNode: (id: string) => void;
}) {
    return (
        <div className="vfs-container" style={{ flex: 1, overflow: 'auto', backgroundColor: '#0A0814' }}>
            {visiblePaths.map((id) => {
                const node = nodes[id];
                if (!node) return null;
                const { heatColor, heatScore } = computeHeatState(id, node.type, heatmap);
                return (
                    <TreeRow
                        key={node.id}
                        node={node}
                        heatColor={heatColor}
                        heatScore={heatScore}
                        onToggleNode={toggleNode}
                        style={{
                            minHeight: '32px',
                            paddingLeft: `${node.depth * 24 + 12}px`,
                        }}
                    />
                );
            })}
        </div>
    );
}

function VirtualizedTreeList({
    visiblePaths,
    nodes,
    heatmap,
    toggleNode,
}: {
    visiblePaths: string[];
    nodes: Record<string, VFSTreeNode>;
    heatmap: Record<string, any>;
    toggleNode: (id: string) => void;
}) {
    const parentRef = React.useRef<HTMLDivElement>(null);
    const virtualizer = useVirtualizer({
        count: visiblePaths.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => 32,
        overscan: 20,
    });

    return (
        <div ref={parentRef} className="vfs-container" style={{ flex: 1, overflow: 'auto', backgroundColor: '#0A0814' }}>
            <div style={{ height: `${virtualizer.getTotalSize()}px`, width: '100%', position: 'relative' }}>
                {virtualizer.getVirtualItems().map((virtualRow) => {
                    const id = visiblePaths[virtualRow.index];
                    const node = nodes[id];
                    if (!node) return null;
                    const { heatColor, heatScore } = computeHeatState(id, node.type, heatmap);
                    return (
                        <TreeRow
                            key={node.id}
                            node={node}
                            heatColor={heatColor}
                            heatScore={heatScore}
                            onToggleNode={toggleNode}
                            style={{
                                position: 'absolute',
                                top: 0,
                                left: 0,
                                width: '100%',
                                height: `${virtualRow.size}px`,
                                transform: `translateY(${virtualRow.start}px)`,
                                paddingLeft: `${node.depth * 24 + 12}px`,
                            }}
                        />
                    );
                })}
            </div>
        </div>
    );
}

export function VfsTreeView({
    triggerRefresh,
    targetKey,
    stableCurrentListingPath,
    outputDir,
}: {
    triggerRefresh: number,
    targetKey: string | null,
    stableCurrentListingPath: string | null,
    outputDir: string,
}) {
    const [nodes, setNodes] = useState<Record<string, VFSTreeNode>>({});
    const [rootPaths, setRootPaths] = useState<string[]>([]);
    const [isInitialLoading, setIsInitialLoading] = useState(true);
    const [heatmap, setHeatmap] = useState<Record<string, any>>({});

    const isTauriRuntime = getIsTauriRuntime();
    const isFixtureMode = !isTauriRuntime && isVfsFixtureMode();

    useEffect(() => {
        if (!targetKey || !isTauriRuntime) return;
        const fetchHeatmap = async () => {
            try {
                const map = await invokeCommand<any>("get_subtree_heatmap", { targetKey });
                if (map && map.entries) {
                    setHeatmap(map.entries);
                }
            } catch (e) {
                console.error("Failed to load subtree heatmap", e);
            }
        };
        fetchHeatmap();
        const interval = setInterval(fetchHeatmap, 5000);
        return () => clearInterval(interval);
    }, [targetKey, isTauriRuntime]);

    const loadChildren = async (parentPath: string, depth: number) => {
        if (isFixtureMode) {
            const parent = normalizeTreePath(parentPath);
            const children = VFS_FIXTURE_ENTRIES
                .map((entry) => ({ ...entry, path: normalizeTreePath(entry.path) }))
                .filter((entry) => isDirectTreeChild(parent, entry.path))
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
                if (parentPath === '') setRootPaths(childIds);
                else if (next[parentPath]) {
                    next[parentPath] = { ...next[parentPath], isLoading: false, childrenLoaded: true, childrenPaths: childIds };
                }
                return next;
            });
            setIsInitialLoading(false);
            return;
        }

        if (!isTauriRuntime) {
            if (parentPath === "") { setRootPaths([]); setIsInitialLoading(false); }
            return;
        }

        try {
            if (parentPath !== '') {
                setNodes(prev => ({ ...prev, [parentPath]: { ...prev[parentPath], isLoading: true } }));
            }
            const children = (await invokeCommand<FileEntry[]>("get_vfs_children", { parentPath }))
                .map((child) => ({
                    ...child,
                    path: normalizeTreePath(child.path),
                }))
                .filter((child) => isDirectTreeChild(parentPath, child.path))
                .sort((a, b) => {
                    if (a.entry_type !== b.entry_type) {
                        return a.entry_type === "Folder" ? -1 : 1;
                    }
                    return a.path.localeCompare(b.path);
                });
            setNodes(prev => {
                const next = { ...prev };
                const childIds: string[] = [];
                children.forEach(child => {
                    const name = child.path.split('/').pop() || child.path;
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
                if (parentPath === '') setRootPaths(childIds);
                else if (next[parentPath]) {
                    next[parentPath] = { ...next[parentPath], isLoading: false, childrenLoaded: true, childrenPaths: childIds };
                }
                return next;
            });
        } catch (e) {
            console.error("Failed to fetch VFS children:", e);
        } finally {
            if (parentPath === '') setIsInitialLoading(false);
        }
    };

    useEffect(() => {
        loadChildren('', 0);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [triggerRefresh, isTauriRuntime, isFixtureMode]);

    const toggleNode = useCallback(async (id: string) => {
        const node = nodes[id];
        if (!node || node.type !== 'Folder') return;

        if (node.isExpanded) {
            setNodes(prev => ({ ...prev, [id]: { ...prev[id], isExpanded: false } }));
        } else {
            if (!node.childrenLoaded) {
                await loadChildren(id, node.depth + 1);
            }
            setNodes(prev => ({ ...prev, [id]: { ...prev[id], isExpanded: true } }));
        }
    }, [nodes]);

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

    const openFolder = async () => {
        if (!isTauriRuntime || !targetKey) return;
        const targetPath = `${outputDir}/targets/${targetKey}/current`;
        try {
            await invokeCommand("open_folder_os", { path: targetPath });
        } catch (e) {
            console.error("Open output folder failed", e);
        }
    };

    const openJSON = async () => {
        if (!isTauriRuntime || !stableCurrentListingPath) return;
        try {
            await invokeCommand("open_folder_os", { path: stableCurrentListingPath });
        } catch (e) {
            console.error("Open JSON failed", e);
        }
    };

    const openTxt = async () => {
        if (!isTauriRuntime || !stableCurrentListingPath) return;
        const txtPath = stableCurrentListingPath.replace('.json', '.txt');
        try {
            await invokeCommand("open_folder_os", { path: txtPath });
        } catch (e) {
            console.error("Open TXT failed", e);
        }
    };

    if (isInitialLoading) {
        return (
            <div style={{ padding: '32px', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-muted)' }}>
                <VibeLoader size={18} /> Building Tree Layout...
            </div>
        );
    }

    if (rootPaths.length === 0) {
        return (
            <div style={{ padding: '32px', textAlign: 'center', color: 'var(--text-muted)' }}>
                No payload map available. Initiate crawl.
            </div>
        );
    }

    return (
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: '300px' }}>
            {targetKey && (
                <div style={{ display: 'flex', padding: '12px', borderBottom: '1px solid var(--border-color)', gap: '12px', flexWrap: 'wrap', backgroundColor: 'rgba(10,8,20,0.5)' }}>
                    <button onClick={openFolder} style={{ display: 'flex', alignItems: 'center', gap: '6px', background: 'transparent', border: '1px solid var(--text-muted)', color: 'var(--text-main)', padding: '4px 12px', borderRadius: '4px', cursor: 'pointer', fontSize: '12px' }}>
                        <ExternalLink size={14} /> Open Folder
                    </button>
                    <button onClick={openTxt} style={{ display: 'flex', alignItems: 'center', gap: '6px', background: 'transparent', border: '1px solid var(--text-muted)', color: 'var(--text-main)', padding: '4px 12px', borderRadius: '4px', cursor: 'pointer', fontSize: '12px' }}>
                        <FileText size={14} /> Export Windows DIR/S
                    </button>
                    <button onClick={openJSON} style={{ display: 'flex', alignItems: 'center', gap: '6px', background: 'transparent', border: '1px solid var(--text-muted)', color: 'var(--text-main)', padding: '4px 12px', borderRadius: '4px', cursor: 'pointer', fontSize: '12px' }}>
                        <Code2 size={14} /> Export JSON
                    </button>
                </div>
            )}

            {!isTauriRuntime ? (
                <SimpleTreeList
                    visiblePaths={visiblePaths}
                    nodes={nodes}
                    heatmap={heatmap}
                    toggleNode={toggleNode}
                />
            ) : (
                <VirtualizedTreeList
                    visiblePaths={visiblePaths}
                    nodes={nodes}
                    heatmap={heatmap}
                    toggleNode={toggleNode}
                />
            )}
        </div>
    );
}
