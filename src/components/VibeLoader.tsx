import React from 'react';
import './VibeLoader.css';

interface VibeLoaderProps {
    size?: number;
    className?: string;
    variant?: 'primary' | 'secondary' | 'accent';
    style?: React.CSSProperties;
}

/**
 * SnoozeSlayer Vibe Architecture Loader
 * Replaces static UI spinners with cinematic 60fps Animated WebP placeholders.
 */
export const VibeLoader: React.FC<VibeLoaderProps> = ({
    size = 18,
    className = '',
    variant = 'primary',
    style = {}
}) => {
    return (
        <div
            className={`vibe-loader-container ${variant} ${className}`}
            style={{ width: size, height: size, ...style }}
        >
            <img
                src="/assets/animations/vibe_spinner_8bit_alpha.webp"
                alt=""
                className="vibe-loader-sequence"
                loading="eager"
                decoding="async"
                onError={(e) => {
                    // If the cinematic asset is missing, cleanly degrade to the CSS ring
                    e.currentTarget.style.display = 'none';
                    e.currentTarget.parentElement?.classList.add('vibe-loader-fallback');
                }}
            />
        </div>
    );
};
