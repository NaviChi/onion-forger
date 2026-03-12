import { render, waitFor, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { VFSExplorer } from './VFSExplorer';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn()
}));

vi.mock('@tanstack/react-virtual', () => ({
    useVirtualizer: vi.fn().mockImplementation((options) => {
        return {
            getVirtualItems: vi.fn(() => {
                // Return all items!
                return new Array(options.count).fill(0).map((_, i) => ({
                    index: i,
                    start: i * 36,
                    size: 36,
                    measureElement: vi.fn()
                }));
            }),
            getTotalSize: vi.fn(() => options.count * 36)
        }
    })
}));

const mockInvoke = invoke as unknown as ReturnType<typeof vi.fn>;

describe('VFSExplorer', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        (window as any).__TAURI_INTERNALS__ = true;
    });

    it('displays loading initially or empty state', async () => {
        mockInvoke.mockResolvedValueOnce([]);
        const { getByText } = render(
            <VFSExplorer
                triggerRefresh={0}
                onDownload={vi.fn()}
                downloadProgress={{}}
            />
        );

        await waitFor(() => {
            expect(getByText('No files detected in virtual file system.')).toBeInTheDocument();
        });
    });

    it('keeps nested files out of the root layer until their folder is expanded', async () => {
        mockInvoke.mockImplementation(async (command, args) => {
            if (command === 'get_vfs_children' && args.parentPath === '') {
                return [
                    { path: '\\folder1\\file1.txt', entry_type: 'File', size_bytes: 1024, raw_url: 'http://test.loc' },
                    { path: '/folder1', entry_type: 'Folder', size_bytes: null, raw_url: '' },
                ];
            }
            if (command === 'get_vfs_children' && (args.parentPath === 'folder1' || args.parentPath === '/folder1')) {
                return [
                    { path: '/folder1/file1.txt', entry_type: 'File', size_bytes: 1024, raw_url: 'http://test.loc' }
                ];
            }
            return [];
        });

        const { getByText, queryByText, getByTestId } = render(
            <VFSExplorer triggerRefresh={1} onDownload={vi.fn()} downloadProgress={{}} />
        );

        await waitFor(() => {
            expect(getByText('folder1')).toBeInTheDocument();
        });
        expect(queryByText('file1.txt')).not.toBeInTheDocument();

        fireEvent.click(getByTestId(`vfs-toggle-${encodeURIComponent('folder1')}`));

        await waitFor(() => {
            expect(getByText('file1.txt')).toBeInTheDocument();
        });
    });

    it('toggles expansion', async () => {
        mockInvoke.mockImplementation(async (command, args) => {
            if (command === 'get_vfs_children') {
                if (args.parentPath === '') {
                    return [{ path: '/folder1', entry_type: 'Folder', size_bytes: null, raw_url: '' }];
                } else if (args.parentPath === 'folder1' || args.parentPath === '/folder1') {
                    return [{ path: '/folder1/child.txt', entry_type: 'File', size_bytes: null, raw_url: '' }];
                }
            }
            return [];
        });

        const { getByText, getByTestId } = render(
            <div style={{ height: '600px' }}>
                <VFSExplorer triggerRefresh={1} onDownload={vi.fn()} />
            </div>
        );

        await waitFor(() => {
            expect(getByText('folder1')).toBeInTheDocument();
        });

        fireEvent.click(getByTestId(`vfs-toggle-${encodeURIComponent('folder1')}`));

        await waitFor(() => {
            expect(getByText('child.txt')).toBeInTheDocument();
        });
    });

    it('selects nodes', async () => {
        const onSelect = vi.fn();
        mockInvoke.mockImplementation(async (command, args) => {
            if (command === 'get_vfs_children' && args.parentPath === '') {
                return [{ path: '/folder1', entry_type: 'Folder', size_bytes: null, raw_url: '' }];
            }
            return [];
        });

        const { getAllByRole, getByText } = render(
            <VFSExplorer triggerRefresh={1} onDownload={vi.fn()} onSelectionChange={onSelect} />
        );

        await waitFor(() => {
            expect(getByText('folder1')).toBeInTheDocument();
        });

        const checkboxes = getAllByRole('checkbox');
        fireEvent.click(checkboxes[0]);
        // Expect selected to be called if it was the top checkbox or node checkbox
        expect(onSelect).toHaveBeenCalled();
    });

    it('triggers download', async () => {
        const onDownload = vi.fn();
        mockInvoke.mockImplementation(async (command, args) => {
            if (command === 'get_vfs_children' && args.parentPath === '') {
                return [{ path: '/test.zip', entry_type: 'File', size_bytes: 0, raw_url: 'http://foo' }];
            }
            return [];
        });

        const { getByTestId, getByText } = render(
            <VFSExplorer triggerRefresh={1} onDownload={onDownload} />
        );

        await waitFor(() => {
            expect(getByText('test.zip')).toBeInTheDocument();
        });

        const dlButton = getByTestId(`vfs-download-${encodeURIComponent('test.zip')}`);
        fireEvent.click(dlButton);
        expect(onDownload).toHaveBeenCalledWith('http://foo', 'test.zip');
    });

});
