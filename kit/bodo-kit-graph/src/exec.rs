//! Running a compiled [`Plan`] on the audio thread and the RT workers.
//!
//! Each block, the [`GraphRunner`] on the audio thread swaps in the newest plan, resets one counter per node to the
//! number of edges coming into it, queues the nodes that have none, and opens a parallel region on its RT pool
//! (project.md section 6.5). Every participant then repeats: take a ready node, sum its inputs, run its processor,
//! and count down the nodes that wait for it, queueing each one that reaches zero. A block costs O(V + E).
//!
//! Results never depend on how many threads run or which thread runs which node: each node sums its own inputs, in
//! the fixed order the compiler chose, so offline renders are bit-identical from run to run.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use atomic_refcell::AtomicRefCell;
use bodo_kit_core::handoff;
use bodo_kit_core::pool::{Participant, RtJob, RtPool, RtPoolConfig, RtPoolError, ThreadGroupSlot};
use bodo_kit_core::queue::Full;
use bodo_kit_core::rt_safe;
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_dsp::gain::mix_into_ramped;

use crate::plan::{Plan, PlanNode};

/// What a processor sees when it runs.
#[derive(Debug)]
pub struct NodeIo<'a> {
    /// Frames in this block.
    pub frames: usize,
    /// Each input port, already summed from its edges, delay-compensated and scaled by the edge gains.
    pub inputs: &'a [AudioBuffer],
    /// The node's output, `frames` long. Its previous contents are unspecified: write every frame.
    pub output: &'a mut AudioBuffer,
}

/// The work a graph node does each block.
pub trait Processor<C>: Send + Sync + 'static {
    /// Processes one block. `context` is shared by every node for the block, for example the transport position.
    ///
    /// Runs on the audio thread or an RT worker, so it must be RT-safe.
    fn process(&mut self, io: &mut NodeIo<'_>, context: &C);
}

/// The job an RT pool runs for a graph: the current plan and the block context.
#[derive(Debug)]
pub struct Executor<P, C> {
    plan: AtomicRefCell<Box<Plan<P>>>,
    context: AtomicRefCell<C>,
    frames: AtomicUsize,
    remaining: AtomicUsize,
    faults: AtomicU64,
}

impl<P: Processor<C>, C: Send + Sync + 'static> RtJob for Executor<P, C> {
    /// Takes ready nodes until every node of the block has run.
    ///
    /// RT-safe.
    #[rt_safe]
    fn work(&self, _participant: Participant) {
        let (Ok(plan), Ok(context)) = (self.plan.try_borrow(), self.context.try_borrow()) else {
            self.faults.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let frames = self.frames.load(Ordering::Relaxed);
        while self.remaining.load(Ordering::Acquire) > 0 {
            match plan.ready.pop() {
                Some(node) => {
                    self.run_node(&plan, node, frames, &context);
                    self.remaining.fetch_sub(1, Ordering::AcqRel);
                }
                None => std::hint::spin_loop(),
            }
        }
    }
}

impl<P: Processor<C>, C: Send + Sync + 'static> Executor<P, C> {
    /// Sums the node's inputs, runs its processor and releases the nodes waiting for it.
    fn run_node(&self, plan: &Plan<P>, index: u32, frames: usize, context: &C) {
        let Some(node) = plan.nodes.get(index as usize) else {
            self.faults.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if !self.gather_inputs(plan, node, frames) {
            self.faults.fetch_add(1, Ordering::Relaxed);
        }
        let started = Instant::now();
        if !self.process(plan, node, frames, context) {
            self.faults.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(time) = plan.times.get(index as usize) {
            let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            time.fetch_add(nanos, Ordering::Relaxed);
        }
        let dependents = plan.dependents.get(node.out_start as usize..node.out_end as usize).unwrap_or(&[]);
        for &dependent in dependents {
            if let Some(next) = plan.nodes.get(dependent as usize)
                && next.pending.fetch_sub(1, Ordering::AcqRel) == 1
                && plan.ready.push(dependent).is_err()
            {
                self.faults.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Fills the node's ports from its incoming edges. Returns false on a borrow conflict, which a correct schedule
    /// never produces.
    fn gather_inputs(&self, plan: &Plan<P>, node: &PlanNode<P>, frames: usize) -> bool {
        let Ok(mut inputs) = node.inputs.try_borrow_mut() else { return false };
        let inputs = &mut *inputs;
        for port in &mut inputs.ports {
            port.set_frames(frames);
            port.clear();
        }
        let edges = plan.edges.get(node.in_start as usize..node.in_end as usize).unwrap_or(&[]);
        let mut ok = true;
        for (edge, state) in edges.iter().zip(inputs.edges.iter_mut()) {
            let wanted = plan.gains.get(edge.gain_slot as usize).map_or(0.0, |g| f32::from_bits(g.load(Ordering::Relaxed)));
            let target = ramp_toward(state.gain, wanted, frames, plan.gain_ramp);
            let Some(port) = inputs.ports.get_mut(edge.port as usize) else { continue };
            let Some(source) = plan.nodes.get(edge.from as usize).and_then(|source| plan.buffers.get(source.output as usize)) else { continue };
            let Ok(source) = source.try_borrow() else {
                ok = false;
                continue;
            };
            match state.scratch.as_mut() {
                None => mix_channels(port, &source, state.gain, target),
                Some(scratch) => {
                    scratch.set_frames(frames);
                    scratch.copy_from(&source);
                    for (channel, line) in scratch.channels_mut().zip(state.lines.iter_mut()) {
                        line.process(channel, edge.delay as usize);
                    }
                    mix_channels(port, scratch, state.gain, target);
                }
            }
            state.gain = target;
        }
        ok
    }

    /// Runs the node's processor, or passes port 0 through when it has none. Returns false on a borrow conflict or a
    /// panic.
    fn process(&self, plan: &Plan<P>, node: &PlanNode<P>, frames: usize, context: &C) -> bool {
        let Some(output) = plan.buffers.get(node.output as usize) else { return false };
        let (Ok(inputs), Ok(mut output), Ok(mut slot)) = (node.inputs.try_borrow(), output.try_borrow_mut(), node.slot.try_borrow_mut()) else {
            return false;
        };
        output.set_frames(frames);
        let slot = &mut *slot;
        match slot.processor.as_mut() {
            Some(processor) if !slot.poisoned => {
                let mut io = NodeIo { frames, inputs: &inputs.ports, output: &mut output };
                // A processor must never panic, but if one does, contain it here rather than lose the audio thread.
                if catch_unwind(AssertUnwindSafe(|| processor.process(&mut io, context))).is_err() {
                    slot.poisoned = true;
                    output.clear();
                    return false;
                }
            }
            Some(_) => output.clear(),
            None => match inputs.ports.first() {
                Some(port) => output.copy_from(port),
                None => output.clear(),
            },
        }
        true
    }
}

/// Where a gain at `current` heading for `wanted` gets to in a block of `frames`: all the way when `ramp` is zero,
/// otherwise at most `frames / ramp` of the full range.
///
/// RT-safe.
fn ramp_toward(current: f32, wanted: f32, frames: usize, ramp: u32) -> f32 {
    if ramp == 0 || current == wanted {
        return wanted;
    }
    let step = frames as f32 / ramp as f32;
    if current < wanted { (current + step).min(wanted) } else { (current - step).max(wanted) }
}

/// Adds `source` into `port`, channel by channel, ramping the gain. A mono source feeds every channel.
fn mix_channels(port: &mut AudioBuffer, source: &AudioBuffer, from: f32, to: f32) {
    let source_channels = source.channels();
    for (index, channel) in port.channels_mut().enumerate() {
        let source_index = if source_channels == 1 { 0 } else { index };
        if source_index < source_channels {
            mix_into_ramped(channel, source.channel(source_index), from, to);
        }
    }
}

/// The audio-thread side of a graph: the RT pool that runs it and the queue new plans arrive on.
pub struct GraphRunner<P: Processor<C>, C: Send + Sync + 'static> {
    pool: RtPool<Executor<P, C>>,
    plans: handoff::Receiver<Box<Plan<P>>>,
}

/// The control-thread side of a graph: sends compiled plans and collects retired ones.
#[derive(Debug)]
pub struct PlanSender<P> {
    plans: handoff::Sender<Box<Plan<P>>>,
}

impl<P> PlanSender<P> {
    /// Sends a plan to the runner, which switches to it at its next block. When the queue is full, the plan comes back.
    pub fn send(&mut self, plan: Plan<P>) -> Result<(), Box<Plan<P>>> {
        self.plans.send(Box::new(plan)).map_err(|Full(plan)| plan)
    }

    /// Drops the plans the runner has retired, with the processors they carried away. Call it regularly.
    ///
    /// Complexity: O(k) in the number of retired plans, plus the cost of dropping them.
    pub fn collect(&mut self) -> usize {
        self.plans.collect()
    }
}

/// Creates a runner that starts with `plan` and `context`, and the sender that feeds it new plans.
///
/// Spawns the pool's workers. Complexity: O(w) in the number of workers.
pub fn runner<P: Processor<C>, C: Send + Sync + 'static>(
    plan: Plan<P>,
    context: C,
    pool: RtPoolConfig,
) -> Result<(GraphRunner<P, C>, PlanSender<P>), RtPoolError> {
    let executor = Executor {
        plan: AtomicRefCell::new(Box::new(plan)),
        context: AtomicRefCell::new(context),
        frames: AtomicUsize::new(0),
        remaining: AtomicUsize::new(0),
        faults: AtomicU64::new(0),
    };
    let (sender, receiver) = handoff::handoff(4);
    let pool = RtPool::new(pool, executor)?;
    Ok((GraphRunner { pool, plans: receiver }, PlanSender { plans: sender }))
}

impl<P: Processor<C>, C: Send + Sync + 'static> GraphRunner<P, C> {
    /// Runs one block of `frames` frames, clamped to the plan's maximum, and returns the frames run. First it switches
    /// to the newest plan, if one arrived, and lets `update` change the block context.
    ///
    /// RT-safe. Complexity: O(V + E) plus the processors' own work.
    #[rt_safe]
    pub fn process(&mut self, frames: usize, update: impl FnOnce(&mut C)) -> usize {
        let job = self.pool.job();
        let frames = {
            let (Ok(mut plan), Ok(mut context)) = (job.plan.try_borrow_mut(), job.context.try_borrow_mut()) else {
                job.faults.fetch_add(1, Ordering::Relaxed);
                return 0;
            };
            self.plans.update_with(&mut plan, |old: &mut Box<Plan<P>>, new: &mut Box<Plan<P>>| new.adopt_from(old));
            update(&mut context);
            let frames = frames.min(plan.max_frames);
            job.frames.store(frames, Ordering::Relaxed);
            while plan.ready.pop().is_some() {}
            for node in &plan.nodes {
                node.pending.store(node.indegree, Ordering::Relaxed);
            }
            for &source in &plan.sources {
                if plan.ready.push(source).is_err() {
                    job.faults.fetch_add(1, Ordering::Relaxed);
                }
            }
            job.remaining.store(plan.nodes.len(), Ordering::Release);
            frames
        };
        self.pool.run();
        frames
    }

    /// Calls `read` with the output node's buffer from the last block, or returns `None` when the plan has no output.
    ///
    /// RT-safe as long as `read` is.
    #[rt_safe]
    pub fn read_output<R>(&self, read: impl FnOnce(&AudioBuffer) -> R) -> Option<R> {
        let plan = self.pool.job().plan.try_borrow().ok()?;
        let node = plan.nodes.get(plan.output? as usize)?;
        let output = plan.buffers.get(node.output as usize)?.try_borrow().ok()?;
        Some(read(&output))
    }

    /// Calls `read` with the output of tap `index` (its place in [`crate::GraphDesc::taps`]) from the last block, or
    /// returns `None` when the plan has no such tap.
    ///
    /// RT-safe as long as `read` is.
    #[rt_safe]
    pub fn read_tap<R>(&self, index: usize, read: impl FnOnce(&AudioBuffer) -> R) -> Option<R> {
        let plan = self.pool.job().plan.try_borrow().ok()?;
        let node = plan.nodes.get(*plan.taps.get(index)? as usize)?;
        let output = plan.buffers.get(node.output as usize)?.try_borrow().ok()?;
        Some(read(&output))
    }

    /// How many taps the current plan has.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn tap_count(&self) -> usize {
        self.pool.job().plan.try_borrow().map_or(0, |plan| plan.taps.len())
    }

    /// How many internal faults have happened: borrow conflicts, which a correct schedule never produces, and
    /// processors that panicked.
    pub fn faults(&self) -> u64 {
        self.pool.job().faults.load(Ordering::Relaxed)
    }

    /// The number of threads that run nodes: the caller plus the workers.
    pub fn participants(&self) -> usize {
        self.pool.participants()
    }

    /// The slot that says which thread group the workers join. Take it before moving the runner to the audio thread.
    pub fn thread_group(&self) -> ThreadGroupSlot {
        self.pool.thread_group()
    }
}
