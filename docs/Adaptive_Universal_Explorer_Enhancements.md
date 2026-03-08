# Adaptive Universal Explorer Enhancements (Tier-4 Intelligent Fallback)

## Observed Patterns from Similar High-Latency Directory Endpoints
- Nginx Autoindex tables (simple <a> + size text)
- CMS + 302 UUID storage redirects with rotating backends
- Next.js SPA with __NEXT_DATA__ JSON or authenticated iframes
- Common paths: /files/, /data/, /storage/, /archive/
- Common signals: 302 load balancing, 403/400 DDoS patterns

## Built-in Intelligence Features
1. Enhanced link scoring (path keywords, file extensions, anchor text)
2. Automatic mode switching (Autoindex / SPA JSON / CMS redirect)
3. Persistent learning from per-target ledger (winning prefixes remembered)
4. Integrated DDoS awareness (demote mirrors on 403/400 patterns)

## Expected Behavior
- Known sites → specialized adapters (maximum speed)
- New/unknown sites → explorer intelligently discovers structure
- Repeat visits → faster due to learned patterns

## Next Implementation
- Add speculative pre-fetch + HTTP/2 multiplexing
- Wire mode detection in root page handler
- Integrate with existing output directory structure and governor
