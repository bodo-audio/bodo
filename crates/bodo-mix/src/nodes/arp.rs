//! The sampler's arpeggiator (project.md section 10.2): the keys held down play one after another, each `rate`
//! ticks apart in time with the song, in the chosen order, over a number of octaves, each note repeated a number of
//! times and lasting `gate` of a step.
//!
//! Everything is in fixed arrays: nothing here allocates.

use bodo_kit_core::rt_safe;
use bodo_model::channel::{ArpDirection, Arpeggiator};

/// The most keys the arpeggiator holds at once; more are ignored.
pub const MAX_HELD: usize = 32;
/// The most notes in one cycle: every held key over four octaves.
const MAX_CYCLE: usize = MAX_HELD * 4;

/// A key held down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Held {
    key: u8,
    velocity: f32,
    /// When it was pressed, for the as-played order.
    order: u64,
}

/// What the arpeggiator does at a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArpAction {
    /// Starts a note.
    On {
        /// The key.
        key: u8,
        /// The velocity, 0 to 1.
        velocity: f32,
        /// The voice id of the note.
        voice: u64,
    },
    /// Ends the note with this voice id.
    Off {
        /// The voice id.
        voice: u64,
    },
}

/// Voice ids of arpeggiated notes have this bit set, so they never meet the sequencer's.
const ARP_VOICE: u64 = 1 << 62;

/// The arpeggiator's state.
#[derive(Clone, Debug)]
pub struct Arp {
    params: Arpeggiator,
    held: [Held; MAX_HELD],
    count: usize,
    pressed: u64,
    step: u64,
    /// When the next step plays, while keys are held.
    next_tick: Option<f64>,
    /// The note sounding and when its gate closes.
    sounding: Option<(u64, f64)>,
    rng: u64,
    /// The block being played: its first tick, ticks per frame and frames.
    block: (f64, f64, usize),
}

impl Arp {
    /// An arpeggiator with `params`, holding nothing.
    pub fn new(params: Arpeggiator) -> Self {
        Self {
            params,
            held: [Held::default(); MAX_HELD],
            count: 0,
            pressed: 0,
            step: 0,
            next_tick: None,
            sounding: None,
            rng: 0x9E37_79B9_7F4A_7C15,
            block: (0.0, 0.0, 0),
        }
    }

    /// Whether it is on.
    pub fn enabled(&self) -> bool {
        self.params.enabled
    }

    fn rate(&self) -> f64 {
        self.params.rate.max(1) as f64
    }

    /// Holds `key` down, pressed at song tick `tick`. The first key starts the pattern on the next step of the grid.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn press(&mut self, key: u8, velocity: f32, tick: f64) {
        if self.held[..self.count].iter().any(|h| h.key == key) || self.count == MAX_HELD {
            return;
        }
        self.pressed += 1;
        self.held[self.count] = Held { key, velocity, order: self.pressed };
        self.count += 1;
        if self.next_tick.is_none() {
            let rate = self.rate();
            self.next_tick = Some((tick / rate - 1e-9).ceil() * rate);
            self.step = 0;
        }
    }

    /// Lets go of `key`. With nothing held the pattern stops, though the last note plays out its gate.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn release(&mut self, key: u8) {
        if let Some(i) = self.held[..self.count].iter().position(|h| h.key == key) {
            self.held.copy_within(i + 1..self.count, i);
            self.count -= 1;
        }
        if self.count == 0 {
            self.next_tick = None;
        }
    }

    /// Lets go of everything and forgets the sounding note, as when the transport stops; returns that note's voice so
    /// it can be released.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn clear(&mut self) -> Option<u64> {
        self.count = 0;
        self.next_tick = None;
        self.sounding.take().map(|(voice, _)| voice)
    }

    /// Starts a block at song tick `start`, `frames` frames of `ticks_per_frame` each. After a jump in the song the
    /// grid lines up with where it is now.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn begin_block(&mut self, start: f64, ticks_per_frame: f64, frames: usize) {
        self.block = (start, ticks_per_frame.max(0.0), frames);
        let rate = self.rate();
        let end = start + frames as f64 * ticks_per_frame;
        if let Some(next) = self.next_tick
            && (next < start - rate || next > end + rate)
        {
            self.next_tick = Some((start / rate - 1e-9).ceil() * rate);
        }
    }

    /// The song tick at `frame` of the block.
    pub fn tick_at(&self, frame: usize) -> f64 {
        self.block.0 + frame as f64 * self.block.1
    }

    fn frame_of(&self, tick: f64) -> usize {
        let (start, per_frame, frames) = self.block;
        // The first frame at or after `tick`, forgiving the rounding of a tick that falls on a frame.
        (((tick - start) / per_frame - 1e-6).ceil().max(0.0) as usize).min(frames.saturating_sub(1))
    }

    /// The tick of the next thing to do in the block, and whether it is a gate closing.
    fn upcoming(&self) -> Option<(f64, bool)> {
        let (start, per_frame, frames) = self.block;
        if per_frame <= 0.0 {
            return None;
        }
        let end = start + frames as f64 * per_frame;
        let off = self.sounding.map(|(_, at)| at).filter(|at| *at < end);
        let step = self.next_tick.filter(|at| *at < end && self.count > 0);
        match (off, step) {
            (Some(off), Some(step)) if off <= step => Some((off, true)),
            (Some(off), None) => Some((off, true)),
            (_, Some(step)) => Some((step, false)),
            (None, None) => None,
        }
    }

    /// The frame of the block where the arpeggiator acts next, if it does in this block.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn due(&self) -> Option<usize> {
        self.upcoming().map(|(tick, _)| self.frame_of(tick))
    }

    /// Does the next thing in the block and returns its frame and action.
    ///
    /// RT-safe. Complexity: O(k log k) in the keys held, for a step.
    #[rt_safe]
    pub fn fire(&mut self) -> Option<(usize, ArpAction)> {
        let (tick, gate_closes) = self.upcoming()?;
        let frame = self.frame_of(tick);
        if gate_closes || self.sounding.is_some() {
            let (voice, _) = self.sounding.take()?;
            return Some((frame, ArpAction::Off { voice }));
        }
        let rate = self.rate();
        self.next_tick = Some(tick + rate);
        let (key, velocity) = self.pick()?;
        let voice = ARP_VOICE | self.step;
        self.step += 1;
        let gate = f64::from(self.params.gate.clamp(0.01, 1.0)) * rate;
        self.sounding = Some((voice, tick + gate));
        Some((frame, ArpAction::On { key, velocity, voice }))
    }

    /// The key and velocity of the current step.
    fn pick(&mut self) -> Option<(u8, f32)> {
        if self.count == 0 {
            return None;
        }
        let mut held = self.held;
        let held = &mut held[..self.count];
        if self.params.direction == ArpDirection::AsPlayed {
            held.sort_unstable_by_key(|h| h.order);
        } else {
            held.sort_unstable_by_key(|h| h.key);
        }
        let octaves = usize::from(self.params.octaves.clamp(1, 4));
        let mut cycle = [(0u8, 0f32); MAX_CYCLE];
        let mut n = 0;
        for octave in 0..octaves {
            for h in held.iter() {
                if let Some(key) = h.key.checked_add(12 * octave as u8).filter(|k| *k <= 127) {
                    cycle[n] = (key, h.velocity);
                    n += 1;
                }
            }
        }
        if n == 0 {
            return None;
        }
        let index = (self.step / u64::from(self.params.repeat.clamp(1, 8))) as usize;
        let up_down = |i: usize| {
            let period = (2 * n).saturating_sub(2).max(1);
            let p = i % period;
            if p < n { p } else { period - p }
        };
        let at = match self.params.direction {
            ArpDirection::Up | ArpDirection::AsPlayed => index % n,
            ArpDirection::Down => n - 1 - index % n,
            ArpDirection::UpDown => up_down(index),
            ArpDirection::DownUp => n - 1 - up_down(index),
            ArpDirection::Random => {
                // xorshift64*, stepped once per note.
                self.rng ^= self.rng >> 12;
                self.rng ^= self.rng << 25;
                self.rng ^= self.rng >> 27;
                (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % n
            }
        };
        cycle.get(at).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action over `blocks` blocks of 100 frames at one tick per frame, as (frame from the start, action).
    fn run(arp: &mut Arp, blocks: usize) -> Vec<(usize, ArpAction)> {
        let mut out = Vec::new();
        for b in 0..blocks {
            arp.begin_block((b * 100) as f64, 1.0, 100);
            while let Some((frame, action)) = arp.fire() {
                out.push((b * 100 + frame, action));
            }
        }
        out
    }

    fn keys(actions: &[(usize, ArpAction)]) -> Vec<(usize, u8)> {
        actions
            .iter()
            .filter_map(|(f, a)| match a {
                ArpAction::On { key, .. } => Some((*f, *key)),
                ArpAction::Off { .. } => None,
            })
            .collect()
    }

    #[test]
    fn up_plays_each_held_key_a_step_apart_over_the_octaves() {
        let mut arp = Arp::new(Arpeggiator { enabled: true, rate: 60, gate: 0.5, octaves: 2, ..Arpeggiator::default() });
        arp.press(64, 1.0, 0.0);
        arp.press(60, 1.0, 0.0);
        let actions = run(&mut arp, 3);
        assert_eq!(keys(&actions), [(0, 60), (60, 64), (120, 72), (180, 76), (240, 60)]);
        // Each note ends half a step after it starts.
        assert!(actions.contains(&(30, ArpAction::Off { voice: ARP_VOICE })));
    }

    #[test]
    fn directions_and_repeats_order_the_notes() {
        let pattern = |direction, repeat| {
            let mut arp = Arp::new(Arpeggiator { enabled: true, rate: 10, gate: 0.5, direction, repeat, ..Arpeggiator::default() });
            for key in [62, 60, 64] {
                arp.press(key, 1.0, 0.0);
            }
            keys(&run(&mut arp, 1)).into_iter().map(|(_, k)| k).take(6).collect::<Vec<_>>()
        };
        assert_eq!(pattern(ArpDirection::Down, 1), [64, 62, 60, 64, 62, 60]);
        assert_eq!(pattern(ArpDirection::UpDown, 1), [60, 62, 64, 62, 60, 62]);
        assert_eq!(pattern(ArpDirection::DownUp, 1), [64, 62, 60, 62, 64, 62]);
        assert_eq!(pattern(ArpDirection::AsPlayed, 1), [62, 60, 64, 62, 60, 64]);
        assert_eq!(pattern(ArpDirection::Up, 2), [60, 60, 62, 62, 64, 64]);
        let random = pattern(ArpDirection::Random, 1);
        assert!(random.iter().all(|k| [60, 62, 64].contains(k)));
    }

    #[test]
    fn a_key_pressed_off_the_grid_waits_for_the_next_step_and_release_stops_it() {
        let mut arp = Arp::new(Arpeggiator { enabled: true, rate: 50, gate: 0.25, ..Arpeggiator::default() });
        arp.press(60, 0.5, 20.0);
        arp.begin_block(0.0, 1.0, 100);
        assert_eq!(arp.due(), Some(50));
        let first = arp.fire();
        assert_eq!(first, Some((50, ArpAction::On { key: 60, velocity: 0.5, voice: ARP_VOICE })));
        arp.release(60);
        let rest: Vec<_> = std::iter::from_fn(|| arp.fire()).collect();
        assert_eq!(rest, [(63, ArpAction::Off { voice: ARP_VOICE })], "the last note plays out its gate, then nothing");
    }
}
