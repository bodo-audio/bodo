//! A pool of real-time worker threads that join the audio thread in parallel regions.
//!
//! The audio thread owns an [`RtPool`] and calls [`RtPool::run`] once per block. `run` opens a parallel region: it
//! wakes the workers, calls [`RtJob::work`] on the calling thread, then returns once every worker that joined the
//! region has left it. A worker that is still waking up when the region closes skips it, so a slow wakeup never holds
//! up the block. The job decides how work is shared, typically by claiming tasks through atomics, and its `work`
//! returns once nothing is left to claim.
//!
//! After a region, workers spin for [`RtPoolConfig::spin`], then park. [`RtPool::run`] only unparks workers that are
//! parked, so back-to-back blocks need no syscalls.
//!
//! Workers can be promoted to real-time priority when they start, and they run with denormals flushed to zero. They
//! also join whatever [`ThreadGroup`] the pool's [`ThreadGroupSlot`] holds, such as the workgroup of a macOS audio
//! device's IO thread, so the OS schedules them against the same deadline as the audio thread. They follow changes
//! to the slot between regions, never inside one.

use crate::rt_safe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::time::SampleRate;

/// How long a parked worker sleeps before it checks for work again, as a backstop to unparking.
const PARK_TIMEOUT: Duration = Duration::from_millis(10);

/// The work an [`RtPool`] runs in each parallel region.
pub trait RtJob: Send + Sync + 'static {
    /// Does this participant's share of the current region, and returns once nothing is left for it to do.
    ///
    /// Runs on the audio thread and on the RT workers, so implementations must be RT-safe. Several participants
    /// call it at the same time, and one participant may call it more than once in a region.
    fn work(&self, participant: Participant);
}

/// Identifies who is calling [`RtJob::work`]: the thread that called [`RtPool::run`], or one of the workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Participant(usize);

impl Participant {
    /// The thread that called [`RtPool::run`].
    pub const CALLER: Self = Self(0);

    /// 0 for the caller, 1 to N for the N workers. Indexes per-participant scratch space of length
    /// [`RtPool::participants`].
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Real-time priority settings for the workers, matching the audio stream they serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtPriority {
    /// The largest block the audio callback renders, in frames.
    pub buffer_frames: u32,
    /// The stream's sample rate.
    pub sample_rate: SampleRate,
}

/// A set of threads the OS schedules together against one deadline, such as the workgroup of a macOS audio device's
/// IO thread.
pub trait ThreadGroup: Send + Sync {
    /// Adds the calling thread to the group, or returns `None` when the OS refuses.
    ///
    /// Talks to the OS; call it outside parallel regions.
    fn join(&self) -> Option<Membership>;
}

/// A thread's place in a [`ThreadGroup`]. Dropping it takes the thread out of the group again, so it stays on the
/// thread that joined.
pub struct Membership {
    leave: Option<Box<dyn FnOnce()>>,
}

impl Membership {
    /// A membership that runs `leave` when dropped.
    pub fn new(leave: impl FnOnce() + 'static) -> Self {
        Self { leave: Some(Box::new(leave)) }
    }
}

impl std::fmt::Debug for Membership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Membership").finish_non_exhaustive()
    }
}

impl Drop for Membership {
    fn drop(&mut self) {
        if let Some(leave) = self.leave.take() {
            leave();
        }
    }
}

/// Which [`ThreadGroup`] an [`RtPool`]'s workers belong to. Take it from [`RtPool::thread_group`] before handing the
/// pool to the audio thread, and set it whenever the audio device changes.
#[derive(Clone, Default)]
pub struct ThreadGroupSlot(Arc<SlotInner>);

#[derive(Default)]
struct SlotInner {
    group: Mutex<Option<Arc<dyn ThreadGroup>>>,
    /// Bumped on every change, so workers notice without taking the lock.
    epoch: AtomicU64,
}

impl std::fmt::Debug for ThreadGroupSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadGroupSlot").field("epoch", &self.epoch()).finish_non_exhaustive()
    }
}

impl ThreadGroupSlot {
    /// Moves the workers to `group`, or out of any group with `None`. Each worker switches after its next region.
    pub fn set(&self, group: Option<Arc<dyn ThreadGroup>>) {
        *self.0.group.lock().unwrap_or_else(PoisonError::into_inner) = group;
        self.0.epoch.fetch_add(1, Ordering::Release);
    }

    fn epoch(&self) -> u64 {
        self.0.epoch.load(Ordering::Acquire)
    }

    fn current(&self) -> Option<Arc<dyn ThreadGroup>> {
        self.0.group.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// How to build an [`RtPool`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtPoolConfig {
    /// Number of worker threads. Zero is allowed: the caller then does all the work.
    pub workers: usize,
    /// Prefix of the worker thread names.
    pub name: String,
    /// How long a worker spins after a region before it parks.
    pub spin: Duration,
    /// Promote the workers to real-time priority. Failing to promote is logged, not fatal.
    pub priority: Option<RtPriority>,
}

impl RtPoolConfig {
    /// A config with `workers` threads named `bodo-rt-N`, a 50 µs spin and normal priority.
    pub fn new(workers: usize) -> Self {
        Self { workers, name: String::from("bodo-rt"), spin: Duration::from_micros(50), priority: None }
    }
}

/// Why an [`RtPool`] could not start.
#[derive(Debug, thiserror::Error)]
pub enum RtPoolError {
    /// The OS refused to create a worker thread.
    #[error("could not spawn RT worker {index}")]
    Spawn {
        /// Index of the worker that failed.
        index: usize,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
}

/// State shared by the caller and the workers.
struct Shared<J> {
    job: J,
    /// Incremented each time a region opens.
    epoch: AtomicU64,
    /// Whether a region is open.
    open: AtomicBool,
    /// Workers between deciding to join a region and leaving it.
    active: AtomicUsize,
    /// One flag per worker, set while it is parked or about to park.
    parked: Box<[AtomicBool]>,
    shutdown: AtomicBool,
    spin: Duration,
    group: ThreadGroupSlot,
}

impl<J: RtJob> Shared<J> {
    /// Blocks until a new region opens, returning `true`, or the pool shuts down, returning `false`.
    fn wait_for_region(&self, parked: &AtomicBool, seen: &mut u64) -> bool {
        let spin_until = Instant::now() + self.spin;
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return false;
            }
            let epoch = self.epoch.load(Ordering::Acquire);
            if epoch != *seen {
                *seen = epoch;
                return true;
            }
            if Instant::now() < spin_until {
                std::hint::spin_loop();
                continue;
            }
            // Announce the park before the last check. `run` bumps the epoch before it reads the flag, so either
            // this check sees the new epoch or `run` sees the flag and unparks this thread.
            parked.store(true, Ordering::SeqCst);
            if self.epoch.load(Ordering::SeqCst) == *seen && !self.shutdown.load(Ordering::SeqCst) {
                thread::park_timeout(PARK_TIMEOUT);
            }
            parked.store(false, Ordering::SeqCst);
        }
    }
}

/// Decrements `active` when a worker leaves a region, even if the job panicked, so `run` never waits forever.
struct LeaveRegion<'a>(&'a AtomicUsize);

impl Drop for LeaveRegion<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn worker_main<J: RtJob>(shared: &Shared<J>, index: usize, priority: Option<RtPriority>, ready: mpsc::Sender<bool>) {
    let rt_handle = priority.and_then(|p| {
        audio_thread_priority::promote_current_thread_to_real_time(p.buffer_frames, p.sample_rate.hz())
            .inspect_err(|err| tracing::warn!(worker = index, %err, "could not promote RT worker to real-time priority"))
            .ok()
    });
    // The pool may already be shutting down if a later worker failed to spawn; nobody listens then.
    let _ = ready.send(rt_handle.is_some());
    drop(ready);

    let Some(parked) = shared.parked.get(index) else { return };
    let participant = Participant(index + 1);
    let mut seen = shared.epoch.load(Ordering::Acquire);
    let mut membership: Option<Membership> = None;
    let mut group_seen = None;
    // The FPU mode is per thread, so set it once for the worker's whole life.
    crate::rt::flush_denormals(|| {
        loop {
            // Joining a group locks and talks to the OS, so it happens here, between regions.
            let group_epoch = shared.group.epoch();
            if group_seen != Some(group_epoch) {
                group_seen = Some(group_epoch);
                // Leave the old group first: a thread can belong to only one.
                drop(membership.take());
                membership = shared.group.current().and_then(|group| group.join());
            }
            if !shared.wait_for_region(parked, &mut seen) {
                break;
            }
            shared.active.fetch_add(1, Ordering::SeqCst);
            let _leave = LeaveRegion(&shared.active);
            // A worker that woke too late finds the region closed and must not touch the job.
            if shared.open.load(Ordering::SeqCst) {
                shared.job.work(participant);
            }
        }
    });
    drop(membership);

    if let Some(handle) = rt_handle
        && let Err(err) = audio_thread_priority::demote_current_thread_from_real_time(handle)
    {
        tracing::warn!(worker = index, %err, "could not demote RT worker");
    }
}

/// A fixed set of worker threads that help the owning thread run an [`RtJob`] in parallel.
///
/// Building and dropping the pool spawn and join threads, so do both off the audio thread.
pub struct RtPool<J: RtJob> {
    shared: Arc<Shared<J>>,
    handles: Vec<JoinHandle<()>>,
    realtime_workers: usize,
}

impl<J: RtJob> RtPool<J> {
    /// Spawns the workers and waits until each has started.
    ///
    /// Complexity: O(w) in the number of workers, plus the cost of spawning them.
    pub fn new(config: RtPoolConfig, job: J) -> Result<Self, RtPoolError> {
        let parked = (0..config.workers).map(|_| AtomicBool::new(false)).collect();
        let shared = Arc::new(Shared {
            job,
            epoch: AtomicU64::new(0),
            open: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            parked,
            shutdown: AtomicBool::new(false),
            spin: config.spin,
            group: ThreadGroupSlot::default(),
        });
        // If a spawn fails, dropping `pool` shuts down the workers that did start.
        let mut pool = Self { shared, handles: Vec::with_capacity(config.workers), realtime_workers: 0 };
        let (ready_tx, ready_rx) = mpsc::channel();
        for index in 0..config.workers {
            let shared = Arc::clone(&pool.shared);
            let ready = ready_tx.clone();
            let priority = config.priority;
            let handle = thread::Builder::new()
                .name(format!("{}-{index}", config.name))
                .spawn(move || worker_main(&shared, index, priority, ready))
                .map_err(|source| RtPoolError::Spawn { index, source })?;
            pool.handles.push(handle);
        }
        drop(ready_tx);
        pool.realtime_workers = ready_rx.iter().filter(|&promoted| promoted).count();
        Ok(pool)
    }

    /// Runs one parallel region: the caller and any workers that join call [`RtJob::work`]. Returns once the
    /// caller's share is done and every worker that joined has left.
    ///
    /// RT-safe. Complexity: O(w) in the number of workers, plus the job's own work.
    #[rt_safe]
    pub fn run(&mut self) {
        let shared = &*self.shared;
        shared.open.store(true, Ordering::SeqCst);
        shared.epoch.fetch_add(1, Ordering::SeqCst);
        for (handle, parked) in self.handles.iter().zip(shared.parked.iter()) {
            if parked.load(Ordering::SeqCst) {
                // Waking a parked worker is a futex wake on Linux, which RealtimeSanitizer flags as a syscall. It
                // never blocks and is the one kernel call this design needs (ADR 0003), so it is exempted here.
                crate::rt::permit(|| handle.thread().unpark());
            }
        }
        shared.job.work(Participant::CALLER);
        // Workers check `open` after announcing themselves in `active`, so once `active` drains no worker can still
        // be inside the job.
        shared.open.store(false, Ordering::SeqCst);
        while shared.active.load(Ordering::SeqCst) != 0 {
            std::hint::spin_loop();
        }
    }

    /// The job. Between calls to [`RtPool::run`] no worker is inside it, so the owner can prepare the next region.
    pub fn job(&self) -> &J {
        &self.shared.job
    }

    /// The caller plus the workers: the number of distinct [`Participant`]s.
    pub fn participants(&self) -> usize {
        self.handles.len() + 1
    }

    /// How many workers reached real-time priority.
    pub fn realtime_workers(&self) -> usize {
        self.realtime_workers
    }

    /// The slot that says which [`ThreadGroup`] the workers belong to.
    pub fn thread_group(&self) -> ThreadGroupSlot {
        self.shared.group.clone()
    }
}

impl<J: RtJob> Drop for RtPool<J> {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        for handle in &self.handles {
            handle.thread().unpark();
        }
        for handle in self.handles.drain(..) {
            // A worker that panicked has already reported it; there is nothing left to clean up.
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Claims task indexes from a shared counter and counts how often each task ran and who ran it.
    struct Counting {
        next: AtomicUsize,
        runs: Vec<AtomicU32>,
        by_participant: Vec<AtomicU32>,
        task_spin: Duration,
    }

    impl Counting {
        fn new(tasks: usize, participants: usize, task_spin: Duration) -> Self {
            Self {
                next: AtomicUsize::new(0),
                runs: (0..tasks).map(|_| AtomicU32::new(0)).collect(),
                by_participant: (0..participants).map(|_| AtomicU32::new(0)).collect(),
                task_spin,
            }
        }
    }

    impl RtJob for Counting {
        fn work(&self, participant: Participant) {
            loop {
                let task = self.next.fetch_add(1, Ordering::Relaxed);
                let Some(runs) = self.runs.get(task) else { return };
                let start = Instant::now();
                while start.elapsed() < self.task_spin {
                    std::hint::spin_loop();
                }
                runs.fetch_add(1, Ordering::Relaxed);
                self.by_participant[participant.index()].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn run_regions(config: RtPoolConfig, tasks: usize, regions: u32, task_spin: Duration) -> RtPool<Counting> {
        let participants = config.workers + 1;
        let mut pool = RtPool::new(config, Counting::new(tasks, participants, task_spin)).unwrap();
        assert_eq!(pool.participants(), participants);
        for region in 1..=regions {
            pool.job().next.store(0, Ordering::Relaxed);
            pool.run();
            for runs in &pool.job().runs {
                assert_eq!(runs.load(Ordering::Relaxed), region, "every task runs exactly once per region");
            }
        }
        pool
    }

    #[test]
    fn every_task_runs_once_per_region() {
        run_regions(RtPoolConfig::new(3), 257, 500, Duration::ZERO);
    }

    #[test]
    fn works_without_workers() {
        let pool = run_regions(RtPoolConfig::new(0), 10, 10, Duration::ZERO);
        assert_eq!(pool.job().by_participant[0].load(Ordering::Relaxed), 100);
    }

    #[test]
    fn parked_workers_wake_up_and_help() {
        let config = RtPoolConfig { spin: Duration::ZERO, ..RtPoolConfig::new(2) };
        let pool = run_regions(config, 32, 300, Duration::from_micros(20));
        let helped: u32 = pool.job().by_participant[1..].iter().map(|n| n.load(Ordering::Relaxed)).sum();
        assert!(helped > 0, "workers never joined a region");
    }

    #[test]
    fn spinning_workers_help() {
        let config = RtPoolConfig { spin: Duration::from_millis(5), ..RtPoolConfig::new(2) };
        let pool = run_regions(config, 32, 300, Duration::from_micros(20));
        let helped: u32 = pool.job().by_participant[1..].iter().map(|n| n.load(Ordering::Relaxed)).sum();
        assert!(helped > 0, "workers never joined a region");
    }

    #[test]
    fn promotion_failure_is_not_fatal() {
        let priority = Some(RtPriority { buffer_frames: 128, sample_rate: SampleRate::HZ_48000 });
        let pool = run_regions(RtPoolConfig { priority, ..RtPoolConfig::new(2) }, 16, 10, Duration::ZERO);
        assert!(pool.realtime_workers() <= 2);
    }

    struct PanicsOnWorkers;

    impl RtJob for PanicsOnWorkers {
        fn work(&self, participant: Participant) {
            if participant != Participant::CALLER {
                panic!("worker job failed");
            }
            // Give the workers time to join so the panic path is exercised. Spin rather than sleep: this runs in
            // the caller's real-time region.
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(2) {
                std::hint::spin_loop();
            }
        }
    }

    #[test]
    fn a_panicking_worker_does_not_hang_the_caller() {
        let mut pool = RtPool::new(RtPoolConfig { spin: Duration::from_millis(50), ..RtPoolConfig::new(2) }, PanicsOnWorkers).unwrap();
        for _ in 0..5 {
            pool.run();
        }
    }

    /// Counts the threads in the group.
    #[derive(Default)]
    struct CountingGroup {
        members: Arc<AtomicUsize>,
        joins: AtomicUsize,
    }

    impl ThreadGroup for CountingGroup {
        fn join(&self) -> Option<Membership> {
            self.joins.fetch_add(1, Ordering::SeqCst);
            self.members.fetch_add(1, Ordering::SeqCst);
            let members = Arc::clone(&self.members);
            Some(Membership::new(move || {
                members.fetch_sub(1, Ordering::SeqCst);
            }))
        }
    }

    #[test]
    fn workers_follow_the_thread_group_between_regions() {
        let config = RtPoolConfig { spin: Duration::from_millis(5), ..RtPoolConfig::new(3) };
        let mut pool = RtPool::new(config, Counting::new(64, 4, Duration::ZERO)).unwrap();
        let first = Arc::new(CountingGroup::default());
        let second = Arc::new(CountingGroup::default());
        let slot = pool.thread_group();
        let settle = |pool: &mut RtPool<Counting>, done: &dyn Fn() -> bool| {
            // Each worker switches after a region it took part in; keep opening regions until all have.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !done() && Instant::now() < deadline {
                pool.job().next.store(0, Ordering::SeqCst);
                pool.run();
                thread::sleep(Duration::from_millis(1));
            }
        };
        slot.set(Some(Arc::clone(&first) as Arc<dyn ThreadGroup>));
        settle(&mut pool, &|| first.members.load(Ordering::SeqCst) == 3);
        assert_eq!(first.members.load(Ordering::SeqCst), 3);
        assert_eq!(first.joins.load(Ordering::SeqCst), 3, "each worker joins once");

        slot.set(Some(Arc::clone(&second) as Arc<dyn ThreadGroup>));
        settle(&mut pool, &|| second.members.load(Ordering::SeqCst) == 3 && first.members.load(Ordering::SeqCst) == 0);
        assert_eq!(first.members.load(Ordering::SeqCst), 0, "workers left the old group");
        assert_eq!(second.members.load(Ordering::SeqCst), 3);

        drop(pool);
        assert_eq!(second.members.load(Ordering::SeqCst), 0, "workers leave when the pool stops");
    }
}
