# Phase 74C: Qilin MultiClient Optimization (Multiplexing Decoupling)

## Background
The previous architecture tightly coupled the number of active Qilin asynchronous workers (`max_concurrent`) to the number of underlying `TorClient` instances (`multi_clients`). When scaling to 16 or 32 circuits to achieve high speed, this forced the runtime to instantiate 16-32 full heavy `TorClient` structs, resulting in:
- High local OS memory usage (often throwing OOM on Mac).
- Excessive Tokio thread pool IO waits and crypto overhead.
- File handle starvation.
- Filesystem permission panics during Arti cache duplication.

## Implementation Fixes
1. **Decoupled Qilin Worker Scale:** Qilin now natively distinguishes between `qilin_workers` (number of async fetch tasks) and `multi_clients` (number of Tor instances).
   - Qilin Workers can scale dynamically up to `CRAWLI_QILIN_WORKERS` (defaulting to min of `circuits_ceiling` and 64).
   - `CRAWLI_MULTI_CLIENTS` now strictly caps at 8 locally, completely eliminating OS-level resource starvation.
   - Result: 64 concurrent asynchronous fetches intelligently multiplex across a lightweight pool of 8 TorClients. This provides maximum throughput with absolutely minimal footprint.
2. **Arti Seed Directory Panic Fixed:** Previous attempts to duplicate `arti/node_0` indiscriminately copied the `/state` subfolder containing `arti_state/lock`, which caused strict Tor permission validations to panic with `Incorrect permissions ... is u=rwx,g=rx,o=rx; must be g-rx,o-rx`. We now strictly limit the copy to the `cache` subfolder, safely preserving the directory consensuses while bypassing state lock panics.

## Real World Application
- `cargo run --example qilin_ijzn_soak -- --circuits-ceiling 64`
- Will log: `[Qilin] Bootstrapping MultiClientPool with 8 TorClients (Multiplexing 64 workers)`
