import { render, waitFor } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { VFSExplorer } from './VFSExplorer';

describe('VFSExplorer', () => {
    it('displays loading initially or empty state', async () => {
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
});
