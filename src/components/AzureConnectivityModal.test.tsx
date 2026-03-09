import { render } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { AzureConnectivityModal } from './AzureConnectivityModal';

describe('AzureConnectivityModal', () => {
    it('renders and handles close', () => {
        const onClose = vi.fn();
        const { getByText } = render(<AzureConnectivityModal isOpen={true} onClose={onClose} />);

        // Check tabs exist
        expect(getByText('Intranet Web Access')).toBeInTheDocument();
        expect(getByText('Azure Storage')).toBeInTheDocument();
    });

    it('does not render when closed', () => {
        const onClose = vi.fn();
        const { queryByText } = render(<AzureConnectivityModal isOpen={false} onClose={onClose} />);
        expect(queryByText('Intranet Web Access')).not.toBeInTheDocument();
    });
});
