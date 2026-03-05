# Qilin Tail-End Performance & Adaptive Healing Architecture

## The Problem: The "128-Second Dropoff"
The user reported that `qilin` slows down dramatically towards the very end of the crawl. Through an architectural audit of the worker pool (`workers.spawn` inside `adapters/qilin.rs`), we have identified a mathematical inevitability causing this bug: **The Isolation of Exponential Backoff.**

1. **The Setup:** We have 60 concurrent Tokio workers pulling from a single `crossbeam_queue`.
2. **The Run:** For the first 34,900 files, the queue is massive. If a worker hits a Tor `429 Too Many Requests` or a TCP drop, it enters a `tokio::time::sleep(2s -> 4s -> 8s -> 16s... up to 128s)` exponential backoff. Because there are 59 other workers actively pulling healthy links from the queue, the user does not notice this single worker sleeping. The UI continues to blaze forward.
3. **The Dropoff (The Bug):** At the very end of the crawl, the `crossbeam_queue` reaches `0`. 59 workers exit their loop because the queue is empty. However, 1 worker is currently holding the *very last URL* and encounters a Tor timeout. 
4. **The Stall:** That single worker is now forced to execute its *entire* 7-pass exponential backoff sequence (2s + 4s + 8s + 16s + 32s + 64s = **126 seconds**) completely alone. The UI freezes at "99% Complete" for 2 minutes because the system is waiting for that single worker to finish its backoff on the final node.

## The Solution: The "Inverted Retry Queue" (Worker-Stealing)

Instead of forcing a worker to sleep *while holding* the URL, we must decouple the backoff timer from the worker thread.

### Current Architecture (Thread-Blocking)
```rust
// A worker claims the URL and refuses to let go until 7 attempts fail.
let url = queue.pop();
for attempt in 1..=7 {
    if fetch(url).fails() {
        sleep(2^attempt seconds).await; // ❌ BAD: This thread is now dead to the world.
    }
}
```

### Proposed Architecture (Adaptive Healing via Shared State)
We implement an **Inverted Retry Queue**. If a worker fails a fetch, it immediately calculates the "next valid retry timestamp", attaches it to the URL, and *pushes it back into a specialized secondary queue*. The worker then immediately grabs a fresh URL from the primary queue.

```rust
struct RetryPayload {
    url: String,
    attempt: u8,
    unlock_timestamp: Instant,
}

// Global state
let primary_queue = SegQueue::new();
let retry_queue = SegQueue::new();

// Worker Loop
loop {
    // 1. Try to pop a normal URL
    if let Some(url) = primary_queue.pop() {
        if fetch(url).fails() {
            // Adaptive Healing: Don't sleep! Push to retry queue with a futuristic unlock time.
            retry_queue.push(RetryPayload {
                url,
                attempt: 1,
                unlock_timestamp: Instant::now() + Duration::from_secs(2),
            });
        }
    } 
    // 2. If primary is empty, check retry queue
    else if let Some(payload) = retry_queue.pop() {
        if Instant::now() >= payload.unlock_timestamp {
            // Time to retry!
            if fetch(payload.url).fails() {
                // Fails again? Push back with attempt = 2 (4 seconds)
                payload.attempt += 1;
                payload.unlock_timestamp = Instant::now() + Duration::from_secs(4);
                retry_queue.push(payload);
            }
        } else {
            // Not ready yet? Put it back instantly.
            retry_queue.push(payload);
        }
    }
}
```

### Why This Fixes The Tail-End Slowdown (Worker-Stealing)
At the end of the crawl, if the final URL fails, it is pushed to the `retry_queue` with a 2-second lock. 
Because it is in a shared queue, **any of the 60 workers** can pick it up. If that specific Tor Circuit (Worker #1) was burned by a firewall, Worker #2 (which has a completely different Tor Entry Relay and Circuit IP) will naturally steal the URL from the retry queue and execute it perfectly.

By implementing this **Adaptive Healing** pattern:
1. We eradicate the 126-second single-thread freeze.
2. We naturally rotate IPs (Circuits) for failing URLs by allowing *other* workers to steal the retry payload.
3. We maximize CPU usage down to the very final millisecond of the crawl.
