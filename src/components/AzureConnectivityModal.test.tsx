import { render } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { AzureConnectivityModal } from './AzureConnectivityModal';

describe('AzureConnectivityModal', () => {
    it('renders and handles close', () => {
        const onClose = vi.fn();
        const onSave = vi.fn();
        const { getByText } = render(<AzureConnectivityModal isOpen={true} onClose={onClose} onSave={onSave} />);

        // Check tabs exist
        expect(getByText('Intranet Web Access')).toBeInTheDocument();
        expect(getByText('Azure Storage')).toBeInTheDocument();
    });

    it('does not render when closed', () => {
        const onClose = vi.fn();
        const onSave = vi.fn();
        const { queryByText } = render(<AzureConnectivityModal isOpen={false} onClose={onClose} onSave={onSave} />);
        expect(queryByText('Intranet Web Access')).not.toBeInTheDocument();
    });
});
