//! Handing values to the audio thread, and getting the old ones back so they are never freed there.
//!
//! The engine compiles a new execution plan off the audio thread and must swap it in without the audio thread
//! allocating or freeing anything (ADR 0003, ADR 0014). [`handoff`] makes a pair:
//!
//! - the [`Sender`], on a control thread, sends new values and drops the ones that come back;
//! - the [`Receiver`], on the audio thread, holds the current value, switches to the newest one at the start of a
//!   block, and sends the old one back.
//!
//! Both directions are bounded queues. The receiver only takes a new value when the return queue has room for the
//! old one, so it never has to drop a value; a full return queue just delays the switch until the sender collects.

use crate::queue::{Consumer, Full, Producer, spsc};
use crate::rt_safe;

/// Creates a handoff that starts with `initial` and holds up to `capacity` values in flight in each direction.
/// A capacity of zero is raised to one.
///
/// Allocates. Complexity: O(1), plus the allocation of the two queues.
pub fn handoff<T: Send>(initial: T, capacity: usize) -> (Sender<T>, Receiver<T>) {
    let capacity = capacity.max(1);
    let (to_rt, incoming) = spsc(capacity);
    let (retired, from_rt) = spsc(capacity);
    (Sender { to_rt, from_rt }, Receiver { current: initial, incoming, retired })
}

/// The control-thread half of a [`handoff`].
#[derive(Debug)]
pub struct Sender<T> {
    to_rt: Producer<T>,
    from_rt: Consumer<T>,
}

impl<T> Sender<T> {
    /// Queues `value` for the receiver. When the queue is full, returns the value so the caller can retry it, or
    /// replace it with something newer, after the receiver has caught up.
    pub fn send(&mut self, value: T) -> Result<(), Full<T>> {
        self.to_rt.push(value)
    }

    /// Drops every value the receiver has retired, and returns how many there were. Call it regularly from the
    /// control or GC thread.
    ///
    /// Complexity: O(k) in the number of retired values, plus the cost of dropping them.
    pub fn collect(&mut self) -> usize {
        let mut count = 0;
        while self.from_rt.pop().is_some() {
            count += 1;
        }
        count
    }

    /// Whether the receiver has been dropped.
    pub fn is_abandoned(&self) -> bool {
        self.to_rt.is_abandoned()
    }
}

/// The audio-thread half of a [`handoff`].
#[derive(Debug)]
pub struct Receiver<T> {
    current: T,
    incoming: Consumer<T>,
    retired: Producer<T>,
}

impl<T> Receiver<T> {
    /// Switches to the newest value sent, retiring each value it replaces. Returns whether the value changed.
    ///
    /// RT-safe: it never drops a value. Complexity: O(k) in the number of values waiting.
    #[rt_safe]
    pub fn update(&mut self) -> bool {
        let mut changed = false;
        while self.retired.free_slots() > 0 {
            let Some(next) = self.incoming.pop() else { break };
            let previous = core::mem::replace(&mut self.current, next);
            if let Err(Full(previous)) = self.retired.push(previous) {
                // Unreachable: the loop checked for a free slot and only this thread pushes. Leaking is the only
                // way to get rid of the value without freeing memory here.
                core::mem::forget(previous);
            }
            changed = true;
        }
        changed
    }

    /// The current value.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn current(&self) -> &T {
        &self.current
    }

    /// The current value, mutably.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn current_mut(&mut self) -> &mut T {
        &mut self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn switches_to_the_newest_value_and_returns_the_old_ones() {
        let (mut tx, mut rx) = handoff(String::from("a"), 4);
        assert!(!rx.update());
        tx.send(String::from("b")).unwrap();
        tx.send(String::from("c")).unwrap();
        assert!(rx.update());
        assert_eq!(rx.current(), "c");
        assert_eq!(tx.collect(), 2, "a and b come back to be dropped");
        assert_eq!(tx.collect(), 0);
    }

    #[test]
    fn never_drops_on_the_receiver_when_the_return_queue_is_full() {
        let marker = Arc::new(());
        let (mut tx, mut rx) = handoff(Arc::clone(&marker), 1);
        tx.send(Arc::clone(&marker)).unwrap();
        assert!(rx.update());
        // The return queue now holds the initial value; the next value has to wait.
        tx.send(Arc::clone(&marker)).unwrap();
        assert!(!rx.update());
        assert_eq!(Arc::strong_count(&marker), 4, "nothing was dropped by the receiver");
        assert_eq!(tx.collect(), 1);
        assert!(rx.update());
        assert_eq!(tx.collect(), 1);
        assert_eq!(Arc::strong_count(&marker), 2, "the marker and the receiver's current value");
    }

    #[test]
    fn a_full_send_queue_hands_the_value_back() {
        let (mut tx, _rx) = handoff(0u32, 1);
        tx.send(1).unwrap();
        assert_eq!(tx.send(2), Err(Full(2)));
    }
}
