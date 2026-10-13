//! Bounded single-producer, single-consumer queues for real-time paths.
//!
//! Every queue on a real-time path is bounded and has an explicit overflow policy. [`spsc`] creates a queue with a
//! fixed capacity, and the producer picks the policy at each push:
//!
//! - [`Producer::push`] rejects: a full queue hands the value back, and the caller decides what to do with it.
//! - [`Producer::push_or_drop`] drops the new value. It exists only for `Copy` values, so dropping never runs a
//!   destructor or frees memory on a real-time thread.
//!
//! Both policies count overflows in a counter that either side can read.
//!
//! Creating a queue allocates, and dropping the last handle frees the buffer, so do both off the audio thread. Values
//! popped on the audio thread must not be dropped there if dropping them frees memory; send them on to a GC thread.

use core::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Creates a queue that holds up to `capacity` values. A queue with capacity zero rejects every push.
///
/// Allocates. Complexity: O(1), plus the allocation of `capacity` slots.
pub fn spsc<T: Send>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let overflows = Arc::new(AtomicU64::new(0));
    (Producer { inner: producer, overflows: Arc::clone(&overflows) }, Consumer { inner: consumer, overflows })
}

/// The value handed back by [`Producer::push`] when the queue is full.
#[derive(Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the queue is full")]
pub struct Full<T>(pub T);

impl<T> fmt::Debug for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Full(..)")
    }
}

/// The sending half of a queue made by [`spsc`].
pub struct Producer<T> {
    inner: rtrb::Producer<T>,
    overflows: Arc<AtomicU64>,
}

impl<T> Producer<T> {
    /// Appends `value`. When the queue is full, counts an overflow and returns the value in [`Full`].
    ///
    /// RT-safe.
    pub fn push(&mut self, value: T) -> Result<(), Full<T>> {
        self.inner.push(value).map_err(|rtrb::PushError::Full(value)| {
            self.overflows.fetch_add(1, Ordering::Relaxed);
            Full(value)
        })
    }

    /// How many values can be pushed before the queue is full.
    ///
    /// RT-safe.
    pub fn free_slots(&self) -> usize {
        self.inner.slots()
    }

    /// The capacity the queue was created with.
    pub fn capacity(&self) -> usize {
        self.inner.buffer().capacity()
    }

    /// How many pushes have overflowed since the queue was created.
    ///
    /// RT-safe.
    pub fn overflows(&self) -> u64 {
        self.overflows.load(Ordering::Relaxed)
    }

    /// Whether the consumer has been dropped.
    ///
    /// RT-safe.
    pub fn is_abandoned(&self) -> bool {
        self.inner.is_abandoned()
    }
}

impl<T: Copy> Producer<T> {
    /// Appends `value`, or drops it and counts an overflow when the queue is full. Returns whether it was queued.
    ///
    /// RT-safe.
    pub fn push_or_drop(&mut self, value: T) -> bool {
        self.push(value).is_ok()
    }
}

impl<T> fmt::Debug for Producer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Producer").field("capacity", &self.capacity()).field("overflows", &self.overflows()).finish()
    }
}

/// The receiving half of a queue made by [`spsc`].
pub struct Consumer<T> {
    inner: rtrb::Consumer<T>,
    overflows: Arc<AtomicU64>,
}

impl<T> Consumer<T> {
    /// Removes and returns the oldest value, or `None` when the queue is empty.
    ///
    /// RT-safe.
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop().ok()
    }

    /// How many values are waiting.
    ///
    /// RT-safe.
    pub fn len(&self) -> usize {
        self.inner.slots()
    }

    /// Whether no values are waiting.
    ///
    /// RT-safe.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The capacity the queue was created with.
    pub fn capacity(&self) -> usize {
        self.inner.buffer().capacity()
    }

    /// How many pushes have overflowed since the queue was created.
    ///
    /// RT-safe.
    pub fn overflows(&self) -> u64 {
        self.overflows.load(Ordering::Relaxed)
    }

    /// Whether the producer has been dropped.
    ///
    /// RT-safe.
    pub fn is_abandoned(&self) -> bool {
        self.inner.is_abandoned()
    }
}

impl<T> fmt::Debug for Consumer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Consumer").field("capacity", &self.capacity()).field("len", &self.len()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivers_in_order() {
        let (mut tx, mut rx) = spsc(4);
        for i in 0..4 {
            tx.push(i).unwrap();
        }
        assert_eq!(rx.len(), 4);
        assert_eq!((0..4).map(|_| rx.pop().unwrap()).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert!(rx.is_empty());
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn push_rejects_and_returns_the_value() {
        let (mut tx, mut rx) = spsc(1);
        tx.push(String::from("kept")).unwrap();
        assert_eq!(tx.free_slots(), 0);
        let Err(Full(back)) = tx.push(String::from("rejected")) else { panic!("queue should be full") };
        assert_eq!(back, "rejected");
        assert_eq!((tx.overflows(), rx.overflows()), (1, 1));
        assert_eq!(rx.pop().as_deref(), Some("kept"));
    }

    #[test]
    fn push_or_drop_counts_dropped_values() {
        let (mut tx, mut rx) = spsc(2);
        let queued: Vec<bool> = (0..5).map(|i| tx.push_or_drop(i)).collect();
        assert_eq!(queued, [true, true, false, false, false]);
        assert_eq!(rx.overflows(), 3);
        assert_eq!((rx.pop(), rx.pop(), rx.pop()), (Some(0), Some(1), None));
    }

    #[test]
    fn zero_capacity_rejects_everything() {
        let (mut tx, rx) = spsc(0);
        assert!(!tx.push_or_drop(1u8));
        assert_eq!(rx.capacity(), 0);
    }

    #[test]
    fn detects_abandonment() {
        let (tx, rx) = spsc::<u8>(1);
        assert!(!rx.is_abandoned());
        drop(tx);
        assert!(rx.is_abandoned());
    }

    #[test]
    fn transfers_across_threads_in_order() {
        const COUNT: u32 = 100_000;
        let (mut tx, mut rx) = spsc(64);
        let producer = std::thread::spawn(move || {
            for mut value in 0..COUNT {
                while let Err(Full(back)) = tx.push(value) {
                    value = back;
                    std::hint::spin_loop();
                }
            }
        });
        let mut expected = 0;
        while expected < COUNT {
            if let Some(value) = rx.pop() {
                assert_eq!(value, expected);
                expected += 1;
            }
        }
        producer.join().unwrap();
    }
}
