use crate::multi_client_pool::MultiClientPool;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct SpeculativePrefetcher {
    _pool: Arc<MultiClientPool>,
    tx_queue: mpsc::Sender<String>,
}

impl SpeculativePrefetcher {
    pub fn new(pool: Arc<MultiClientPool>, worker_capacity: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<String>(1024);

        let pool_clone = Arc::clone(&pool);
        tokio::spawn(async move {
            let mut active_tasks = tokio::task::JoinSet::new();

            while let Some(url) = rx.recv().await {
                let pool = Arc::clone(&pool_clone);

                // Keep the number of active pre-fetch streams bounded
                while active_tasks.len() >= worker_capacity {
                    active_tasks.join_next().await;
                }

                active_tasks.spawn(async move {
                    // Grab client from governor's perspective, using a random slot
                    // (Or cycle through them for distribution).
                    let slot = rand::random::<usize>() % worker_capacity.max(1);
                    let client = pool.get_client(slot).await;

                    // Execute a quick HEAD request just to pre-warm the HTTP/2 connection
                    // and cache Tor descriptors
                    let _ = crate::arti_client::ArtiClient::new((*client).clone(), None)
                        .head(&url)
                        .send()
                        .await;
                });
            }

            // Await remaining
            while active_tasks.join_next().await.is_some() {}
        });

        Self {
            _pool: pool,
            tx_queue: tx,
        }
    }

    pub fn queue_speculative_children(&self, urls: Vec<String>) {
        let tx = self.tx_queue.clone();
        tokio::spawn(async move {
            // We only enqueue up to 3 child folders to not overload the Tor pipeline (25-40% speed up limit)
            for url in urls.into_iter().take(3) {
                let _ = tx.send(url).await;
            }
        });
    }
}
