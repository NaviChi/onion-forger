//! Phase 119: Work-Stealing Retry Queue for all adapters.
//!
//! Implements the "Inverted Retry Queue" pattern from Adaptive_Healing_Architecture.md:
//! - On failure, URLs are re-queued with an unlock timestamp instead of blocking the worker.
//! - ANY idle worker can steal retry items, naturally rotating circuits for failed URLs.
//! - Eliminates the 126-second tail-stall documented in the whitepaper.

use std::time::{Duration, Instant};

/// A URL that failed and should be retried after `unlock_at`.
pub struct RetryPayload {
    pub url: String,
    pub attempt: u8,
    pub unlock_at: Instant,
}

/// Shared retry queue using lock-free SegQueue for zero-contention access.
pub type RetryQueue = crossbeam_queue::SegQueue<RetryPayload>;

/// Create a new empty retry queue.
pub fn new_retry_queue() -> RetryQueue {
    crossbeam_queue::SegQueue::new()
}

/// Push a failed URL into the retry queue with exponential backoff.
/// Returns `true` if the URL was re-queued, `false` if max attempts exceeded.
pub fn requeue_with_backoff(
    retry_q: &RetryQueue,
    url: String,
    current_attempt: u8,
    max_attempts: u8,
) -> bool {
    let next_attempt = current_attempt + 1;
    if next_attempt >= max_attempts {
        return false; // Give up
    }
    let backoff_secs = match next_attempt {
        1 => 2,
        2 => 5,
        _ => 10,
    };
    retry_q.push(RetryPayload {
        url,
        attempt: next_attempt,
        unlock_at: Instant::now() + Duration::from_secs(backoff_secs),
    });
    true
}

/// Try to pop a ready retry item. If the item isn't ready yet, pushes it back.
/// Returns `Some((url, attempt))` if a retry is ready, `None` otherwise.
pub fn try_pop_retry(retry_q: &RetryQueue) -> Option<(String, u8)> {
    if let Some(payload) = retry_q.pop() {
        if Instant::now() >= payload.unlock_at {
            Some((payload.url, payload.attempt))
        } else {
            // Not ready — push back
            retry_q.push(payload);
            None
        }
    } else {
        None
    }
}

/// Classify HTTP status codes and decide recovery action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// Re-queue with backoff (timeout, body read failure)
    Retry,
    /// Server overloaded — re-queue with longer backoff
    Backoff,
    /// Permanent failure — don't retry (404, 403, 401)
    Skip,
    /// Connection dead — don't retry this URL
    Abandon,
}

/// Classify an HTTP status code into a recovery action.
pub fn classify_http_status(status: u16) -> FailureAction {
    match status {
        404 => FailureAction::Skip,
        403 | 401 => FailureAction::Skip,
        429 | 503 => FailureAction::Backoff,
        500 | 502 | 504 => FailureAction::Retry,
        _ if status >= 400 => FailureAction::Retry,
        _ => FailureAction::Skip, // Success statuses shouldn't reach here
    }
}

/// Push a URL for retry based on failure classification.
pub fn handle_failure(
    retry_q: &RetryQueue,
    url: String,
    current_attempt: u8,
    action: FailureAction,
) -> bool {
    match action {
        FailureAction::Skip | FailureAction::Abandon => false,
        FailureAction::Retry => requeue_with_backoff(retry_q, url, current_attempt, 3),
        FailureAction::Backoff => {
            let next = current_attempt + 1;
            if next >= 3 {
                return false;
            }
            retry_q.push(RetryPayload {
                url,
                attempt: next,
                unlock_at: Instant::now() + Duration::from_secs(8),
            });
            true
        }
    }
}
