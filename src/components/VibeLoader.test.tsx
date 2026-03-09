import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { VibeLoader } from './VibeLoader';

describe('VibeLoader', () => {
    it('renders with default props', () => {
        const { container } = render(<VibeLoader />);
        const div = container.firstChild as HTMLElement;
        expect(div).toHaveClass('vibe-loader-container');
        expect(div).toHaveClass('primary');
        expect(div).toHaveStyle({ width: '18px', height: '18px' });

        const img = screen.getByRole('presentation', { hidden: true });
        expect(img).toHaveAttribute('src', '/assets/animations/vibe_spinner_8bit_alpha.webp');
    });

    it('renders with custom size, variant and className', () => {
        const { container } = render(<VibeLoader size={32} variant="secondary" className="custom" />);
        const div = container.firstChild as HTMLElement;
        expect(div).toHaveClass('secondary');
        expect(div).toHaveClass('custom');
        expect(div).toHaveStyle({ width: '32px', height: '32px' });
    });

    it('falls back to CSS ring if image fails to load', () => {
        const { container } = render(<VibeLoader />);
        const div = container.firstChild as HTMLElement;
        const img = screen.getByRole('presentation', { hidden: true });

        // Ensure image is visible initially
        expect(img).not.toHaveStyle({ display: 'none' });
        expect(div).not.toHaveClass('vibe-loader-fallback');

        // Trigger onError
        fireEvent.error(img);

        expect(img).toHaveStyle({ display: 'none' });
        expect(div).toHaveClass('vibe-loader-fallback');
    });
});
