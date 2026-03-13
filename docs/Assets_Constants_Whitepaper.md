# Assets and Constants Reference

## UI Constants & Global Scales
- **Glassmorphism Base Layer**: `background: rgba(15, 23, 42, 0.28); backdrop-filter: blur(32px) saturate(180%);`
- **Noise Texture Overlay**: Standard 512x512 SVG noise pattern mapped to full screen at `opacity: 0.04`.
- **Primary Typography**: 'Inter' for UI chrome, 'JetBrains Mono' for technical readouts and JSON data points.
- **Color Palette (Neon Cyber)**: 
  - Primary Base: `#0f172a`
  - Highlighting: `#00f0ff` (Cyan)
  - Critical/Error: `#ff003c` (Crimson)
  - Success/Valid Path: `#39ff14` (Neon Green)

## Tor Crawler Assets
- **Node Loading Animation**: A dynamic graph utilizing WebGL or Canvas 2D to represent the 12 Tor Daemons as pulsing nodes.
- **Crawl Result Virtualized List**: Uses custom high-performance scrollbar styling matching the glass aesthetic.

*Prevention Rule*: Never load generic PNGs. Procedurally generate assets using CSS or SVG to minimize bundle size and RAM footprint, in accordance with our Hardware Squeezing mandate.

*Last Updated: 2026-03-12*
