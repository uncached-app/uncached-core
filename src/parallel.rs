use jwalk::rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::{Arc, OnceLock};

/// One process-wide worker pool serves every parallel filesystem pass (sizing
/// walks and secure-open collection). Building a pool per operation multiplied
/// threads by the number of targets processed concurrently.
pub(crate) fn worker_pool() -> Option<Arc<ThreadPool>> {
    static POOL: OnceLock<Option<Arc<ThreadPool>>> = OnceLock::new();
    POOL.get_or_init(|| {
        ThreadPoolBuilder::new()
            .num_threads(worker_count())
            .thread_name(|index| format!("uncached-worker-{index}"))
            .build()
            .ok()
            .map(Arc::new)
    })
    .clone()
}

fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 8))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{worker_count, worker_pool};

    #[test]
    fn worker_count_stays_within_bounds() {
        let count = worker_count();

        assert!((1..=8).contains(&count));
    }

    #[test]
    fn worker_pool_is_shared_between_calls() {
        let first = worker_pool();
        let second = worker_pool();

        match (first, second) {
            (Some(first), Some(second)) => assert!(std::sync::Arc::ptr_eq(&first, &second)),
            (None, None) => {}
            _ => panic!("pool availability must be stable"),
        }
    }
}
