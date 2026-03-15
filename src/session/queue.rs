use std::collections::BinaryHeap;
use std::cmp::Ordering;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore, OwnedSemaphorePermit};

use super::Priority;

/// A pending request in the priority queue
struct PendingRequest {
    priority: Priority,
    sequence: u64,
    waker: tokio::sync::oneshot::Sender<OwnedSemaphorePermit>,
}

impl PartialEq for PendingRequest {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}

impl Eq for PendingRequest {}

impl PartialOrd for PendingRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PendingRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first, then earlier sequence first (FIFO within same priority)
        self.priority
            .cmp(&other.priority)
            .then(other.sequence.cmp(&self.sequence))
    }
}

/// Concurrency-limited priority queue for session requests
pub struct SessionQueue {
    semaphore: Arc<Semaphore>,
    pending: Mutex<BinaryHeap<PendingRequest>>,
    sequence: Mutex<u64>,
}

#[allow(dead_code)]
impl SessionQueue {
    pub fn new(max_concurrent: usize) -> Self {
        SessionQueue {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            pending: Mutex::new(BinaryHeap::new()),
            sequence: Mutex::new(0),
        }
    }

    /// Acquire a permit, waiting if necessary. Higher priority requests go first.
    pub async fn acquire(&self, priority: Priority) -> OwnedSemaphorePermit {
        // Try to acquire immediately
        if let Ok(permit) = self.semaphore.clone().try_acquire_owned() {
            return permit;
        }

        // Queue up and wait
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut seq = self.sequence.lock().await;
            *seq += 1;
            let request = PendingRequest {
                priority,
                sequence: *seq,
                waker: tx,
            };
            self.pending.lock().await.push(request);
        }

        // Spawn a task to drain the queue as permits become available
        self.drain_queue().await;

        // Wait for our turn
        rx.await.expect("queue permit sender dropped")
    }

    async fn drain_queue(&self) {
        let semaphore = self.semaphore.clone();
        let pending = &self.pending;

        loop {
            let mut queue = pending.lock().await;
            if queue.is_empty() {
                break;
            }

            match semaphore.clone().try_acquire_owned() {
                Ok(permit) => {
                    if let Some(request) = queue.pop() {
                        let _ = request.waker.send(permit);
                    }
                }
                Err(_) => break,
            }
        }
    }

    /// Get current queue depth
    pub async fn queue_depth(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Get available permits
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}
