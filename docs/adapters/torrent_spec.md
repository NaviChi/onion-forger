# BitTorrent / Magnet Adapter Specification (Phase 52A)

## Architecture Overview
The Torrent handler provides clearnet-only fallback downloading and local `.torrent` file parsing capabilities. It is utilized heavily for tracking architectures like **Akira**, which offload payload delivery to peer-to-peer decentralized swarms.

## Core Mechanisms
1. **URL Detection (`is_magnet_link`, `is_torrent_file`)**
   - Safely intercepts `magnet:?` URIs and `.torrent` file paths natively through the generic command pallet without dropping to HTTP routing.
2. **Parsing Engine (`lava_torrent` and `magnet_url`)**
   - Automatically decodes Bencode `.torrent` metadata into Crawli-native generic `FileEntry` arrays.
   - Triggers Magnet display name extractions.
3. **Download Engine (`librqbit`)**
   - Spins up native high-throughput P2P downloads and reports live `torrent_download_progress` through Tauri standard events.
   - Clearnet-native constraint to preserve Tor capacity.

## Prevention Rules
- **PR-TORRENT-001**: Never route BitTorrent packets through the Tor daemon.
- **PR-TORRENT-002**: Always drop/reject local `.torrent` parsing on payloads > 10MB to prevent memory-exhaustion.
