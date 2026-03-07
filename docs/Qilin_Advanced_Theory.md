> **Last Updated:** 2026-03-06T13:05 CST

# Qilin Advanced Theory

## 1. Canonical Test Target
Use this authorized CMS URL for Qilin testing and documentation:

`http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed`

Expected full-tree order of magnitude from prior operator knowledge:
- about `35,069` total nodes

## 2. Validated Qilin Flow
Qilin is a two-stage crawl target:

1. CMS/blog page at `/site/view?uuid=...`
2. `Watch data` handoff to a storage onion host and resolved backend UUID
3. real QData listing crawl on that resolved storage URL

Important consequences:
- the original CMS UUID is not the crawl root
- the resolved storage UUID may differ from the CMS UUID
- storage hosts can rotate between sessions

## 3. What Was Actually Broken
Earlier failures were layered:

1. CMS-to-storage handoff was not always followed correctly.
2. Direct Arti redirect-follow behavior differed from the older HTTP path.
3. Runtime bootstrap could consume the whole test window before crawl work began.
4. Child-folder traversal was reconstructing URLs too manually and needed canonical joining.

## 4. What Is Fixed Now

### 4.1 Discovery
- The adapter follows `Watch data` explicitly.
- Stage A only accepts a candidate when the response body looks like a live QData listing.
- `QilinNodeCache` preserves discovered storage nodes and supports bounded standby routing.

### 4.2 Runtime startup
- Crawl work starts after a quorum of ready clients instead of waiting for the full requested pool.
- Background bootstrap grows the client pool afterward.
- The frontier now reads from the live client pool, so background growth is actually usable.

### 4.3 Recursive traversal
- Child URLs are now built with `Url::join`.
- The parser uses the resolved final URL as the recursion base.
- Limited child queue/fetch/parse/failure diagnostics are emitted so we can inspect the first recursive layer clearly.
- Timeout/circuit-heavy child folders now have a bounded degraded retry lane instead of competing directly with the main retry path.

## 5. Current Measured State

### Short 90-second authorized comparison

| Runtime | Unique Entries | Files | Folders | Notes |
|---|---:|---:|---:|---|
| `native` | 1693 | 1212 | 481 | timed out at 90s, recursive crawl confirmed |
| `torforge` | 973 | 685 | 288 | timed out at 90s, recursive crawl confirmed |

### Meaning
- The adapter is no longer returning `0/0`.
- Recursive crawl is functioning.
- The remaining gap is throughput and long-run stability, not root-page detection.

### Five-minute authorized comparison

| Runtime | Unique Entries | Files | Folders | Notes |
|---|---:|---:|---:|---|
| `native` | 18297 | 16891 | 1406 | timed out at 300s, recursive crawl sustained |
| `torforge` | 18313 | 16888 | 1425 | timed out at 300s, recursive crawl sustained |

Meaning:
- the two runtimes are now effectively tied on five-minute crawl yield
- `torforge` is no longer materially behind on this canonical target
- the next gains need to come from long-tail recursive efficiency rather than just runtime switching

## 6. Current Bottleneck
The current blocker is not the root parser. It is the long recursive tail:

- deeper child folders still produce intermittent connect failures
- short windows do not fully expose which runtime wins long-run
- the crawler now needs longer observation windows and better time-slope measurement

## 7. Runtime Policy
Current recommended Qilin runtime policy:

- start crawl after `3-5` ready clients
- grow toward `6-8` active clients in the background
- keep `12` as a future ceiling, not the default active target
- keep one validated storage route plus a small standby set
- treat raw circuit count as a budget ceiling, not as live HTML worker width

## 8. Theories Worth Testing Next

### 8.1 Longer listing-only soak
Now that recursion works, the next meaningful benchmark is:
- same canonical target
- `5` minutes minimum
- compare discovered-entry slope over time for `native` vs `torforge`

### 8.2 Discovery lane isolation
Keep CMS/storage discovery lightweight and separate from heavy recursive enumeration.

### 8.3 Failure-aware queue shaping
If a child folder fails with connect errors repeatedly:
- keep it in a bounded retry lane
- do not let it dominate the whole worker pool

### 8.4 Avoid naive oversubscription
A controlled `2x` client-multiplex experiment underperformed badly on the same target. For Qilin listing traffic, higher in-flight pressure per client is not a safe default speed lever.

### 8.5 Persistent bad-subtree heatmap
The crawler now has an experimental persistent subtree heatmap implementation that clusters timeout/circuit-heavy prefixes across runs.

Current policy:
- keep it disabled by default
- enable only for explicit testing
- remove or redesign it if repeated benchmarks do not show a measurable gain

Implementation note:
- live subtree shaping is now controlled by `CRAWLI_QILIN_SUBTREE_SHAPING`
- cross-run persistence is controlled separately by `CRAWLI_QILIN_SUBTREE_HEATMAP`

## 9. Theories Deferred
- exact relay selection from the consensus file
- replacing Arti’s path policy with custom relay routing inside `Crawli`
- assuming HTTP/2 or single-socket multiplexing is a win before target evidence supports it

## 10. Lessons Learned
- `Watch data` is the first-class ingress path.
- Storage nodes are rotating infrastructure, not fixed roots.
- Root-page success is necessary but not sufficient; recursive correctness must be validated separately.
- Canonical URL joining matters on QData trees with encoded spaces and punctuation.
- Worker-local client reuse matters more than blunt worker inflation.
- Persistent recovery heuristics must earn their keep in benchmarks or stay off.
