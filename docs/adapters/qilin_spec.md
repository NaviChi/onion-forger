> **Last Updated:** 2026-03-06T13:05 CST

# Qilin Adapter Flight Manual

Adapter ID: `qilin`

## 1. Canonical Test URL
Use this authorized CMS URL as the canonical Qilin documentation and test target:

`http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/`

## 2. Matching Rule
Qilin is identified by QData/CMS markers such as:
- `QData`
- `Data browser`
- `_csrf-blog`
- `item_box_photos`
- QData-style onion value fields in the body

## 3. Ingress Rule
The CMS page is only the launcher surface.

The adapter must:
1. fingerprint the CMS page
2. follow the `Watch data` handoff
3. resolve the storage onion host
4. preserve the resolved backend UUID
5. crawl the resulting QData listing URL

Do not treat the original `/site/view?uuid=...` page as the directory root.

## 4. Storage Node Policy
Qilin storage nodes are rotating infrastructure.

Current policy:
- persist discovered nodes in `QilinNodeCache`
- validate a Stage A candidate only if the body looks like a live QData listing
- choose one primary storage route
- keep a small bounded standby set
- fail over only on classified timeout/circuit/throttle pressure

## 5. Crawl Policy
Current crawl behavior:
- bootstrap after a quorum of ready clients instead of waiting for the full requested pool
- use adaptive page governance for HTML enumeration
- keep the user-selected circuit count as a budget ceiling
- stream entries into sled VFS continuously

## 6. Recursive Parsing Rule
For QData tables:
- resolve child links with `Url::join`
- derive display names from decoded last path segments
- use the resolved final URL as the recursion base

Do not manually reconstruct child URLs with string concatenation when a canonical join is available.

## 7. Diagnostics
Current high-value diagnostics:
- root fetch
- root listing markers
- root parse counts
- limited child queue/fetch/parse/failure logs

These diagnostics are intentionally capped so long runs stay readable.

## 8. Current Known State
Validated live behavior now includes:
- root parsing
- recursive child-folder traversal
- intermittent deeper child-folder connect failures

Short authorized 90-second soaks currently show:
- `native`: 1693 unique entries
- `torforge`: 973 unique entries

That means Qilin recursion works now. The remaining issue is long-run throughput and deeper recursive stability.

## 9. Prevention Rules
- Never use the CMS UUID page as the crawl root once a real storage listing has been resolved.
- Never accept a storage-node candidate only from redirect shape; validate the body too.
- Never reconstruct QData child URLs with raw string concatenation when URL join can preserve canonical encoding.
- Never treat the circuit ceiling as the live HTML worker count for Qilin.
- Never requeue missing folders forever; reconciliation must have a no-progress escape path.
