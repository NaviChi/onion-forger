import { render, waitFor } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { VfsTreeView } from './VfsTreeView';

describe('VfsTreeView', () => {
    it('displays loading indicator initially', async () => {
        const { getByText } = render(<VfsTreeView triggerRefresh={0} targetKey={null} stableCurrentListingPath={null} outputDir="" />);
        expect(getByText('No payload map available. Initiate crawl.')).toBeInTheDocument();

        // Let the mock tauri API settle
        await waitFor(() => {
            expect(getByText('No payload map available. Initiate crawl.')).toBeInTheDocument();
        });
    });
});
