//! Handing values to the audio thread, and getting the old ones back so they are never freed there.
//!
//! The engine compiles a new execution plan off the audio thread and must swap it in without the audio thread
//! allocating or freeing anything (ADR 0003, ADR 0014). [`handoff`] makes a pair:
//!
//! - the [`Sender`], on a control thread, sends new values and drops the ones that come back;
//! - the [`Receiver`], on the audio thread, swaps the newest value into the slot that holds the current one, at the
//!   start of a block, and sends the old one back.
//!
//! The current value lives wherever the audio side keeps it, for example inside an RT pool job that worker threads
//! share; the receiver only moves values in and out of it.
//!
//! Both directions are bounded queues. The receiver only takes a new value when the return queue has room for the
//! old one, so it never has to drop a value; a full return queue just delays the switch until the sender collects.

use crate::queue::{Consumer, Full, Producer, spsc};
use crate::rt_safe;

/// Creates a handoff that holds up to `capacity` values in flight in each direction. A capacity of zero is raised to
/// one.
///
/// Allocates. Complexity: O(1), plus the allocation of the two queues.
pub fn handoff<T: Send>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let capacity = capacity.max(1);
    let (to_rt, incoming) = spsc(capacity);
    let (retired, from_rt) = spsc(capacity);
    (Sender { to_rt, from_rt }, Receiver { incoming, retired })
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
    incoming: Consumer<T>,
    retired: Producer<T>,
}

impl<T> Receiver<T> {
    /// Swaps the newest value sent into `current`, retiring each value it replaces. Returns whether `current`
    /// changed.
    ///
    /// RT-safe: it never drops a value. Complexity: O(k) in the number of values waiting.
    #[rt_safe]
    pub fn update(&mut self, current: &mut T) -> bool {
        self.update_with(current, |_, _| {})
    }

    /// Like [`Receiver::update`], but first calls `adopt(old, new)` for each replacement, so the new value can take
    /// over state from the old one, such as processors that must outlive a plan.
    ///
    /// RT-safe as long as `adopt` is. Complexity: O(k) in the number of values waiting, plus the cost of `adopt`.
    #[rt_safe]
    pub fn update_with(&mut self, current: &mut T, mut adopt: impl FnMut(&mut T, &mut T)) -> bool {
        let mut changed = false;
        while self.retired.free_slots() > 0 {
            let Some(mut next) = self.incoming.pop() else { break };
            adopt(current, &mut next);
            let previous = core::mem::replace(current, next);
            if let Err(Full(previous)) = self.retired.push(previous) {
                // Unreachable: the loop checked for a free slot and only this thread pushes. Leaking is the only
                // way to get rid of the value without freeing memory here.
                core::mem::forget(previous);
            }
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn switches_to_the_newest_value_and_returns_the_old_ones() {
        let (mut tx, mut rx) = handoff(4);
        let mut current = String::from("a");
        assert!(!rx.update(&mut current));
        tx.send(String::from("b")).unwrap();
        tx.send(String::from("c")).unwrap();
        assert!(rx.update(&mut current));
        assert_eq!(current, "c");
        assert_eq!(tx.collect(), 2, "a and b come back to be dropped");
        assert_eq!(tx.collect(), 0);
    }

    #[test]
    fn never_drops_on_the_receiver_when_the_return_queue_is_full() {
        let marker = Arc::new(());
        let (mut tx, mut rx) = handoff(1);
        let mut current = Arc::clone(&marker);
        tx.send(Arc::clone(&marker)).unwrap();
        assert!(rx.update(&mut current));
        // The return queue now holds the initial value; the next value has to wait.
        tx.send(Arc::clone(&marker)).unwrap();
        assert!(!rx.update(&mut current));
        assert_eq!(Arc::strong_count(&marker), 4, "nothing was dropped by the receiver");
        assert_eq!(tx.collect(), 1);
        assert!(rx.update(&mut current));
        assert_eq!(tx.collect(), 1);
        assert_eq!(Arc::strong_count(&marker), 2, "the marker and the current value");
    }

    #[test]
    fn a_full_send_queue_hands_the_value_back() {
        let (mut tx, _rx) = handoff::<u32>(1);
        tx.send(1).unwrap();
        assert_eq!(tx.send(2), Err(Full(2)));
    }

    #[test]
    fn adopt_moves_state_from_the_old_value() {
        let (mut tx, mut rx) = handoff(2);
        let mut current = (String::from("state"), 1);
        tx.send((String::new(), 2)).unwrap();
        assert!(rx.update_with(&mut current, |old, new| core::mem::swap(&mut old.0, &mut new.0)));
        assert_eq!(current, (String::from("state"), 2));
        assert_eq!(tx.collect(), 1);
    }
}
