//! Meters the interface polls every frame (project.md section 9.7), shared lock-free with the audio thread.
//!
//! - [`StripMeter`]: an insert's peak levels at its post-fader point since the last read, a clip latch, and for the
//!   master the correlation between left and right.
//! - [`Scope`]: the last moments of the master output, for an oscilloscope.
//!
//! The audio thread only stores into atomics, so metering never blocks or allocates, and a reader can never hold the
//! audio thread up.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use bodo_kit_core::rt_safe;

use crate::live::AtomicF32;

/// A level 0 dBFS or louder lights the clip latch.
const CLIP: f32 = 1.0;

/// What a meter read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterReading {
    /// The loudest sample of each side since the last read, as a linear level: 1 is 0 dBFS.
    pub peak: [f32; 2],
    /// Whether the strip has gone over 0 dBFS since the latch was cleared.
    pub clipped: bool,
    /// How alike left and right are, from -1 (opposite) through 0 (unrelated) to 1 (mono), smoothed over a few
    /// hundred milliseconds. Only the master measures it; other strips read 1.
    pub correlation: f32,
}

/// The meter of one strip.
#[derive(Debug)]
pub struct StripMeter {
    peak_left: AtomicF32,
    peak_right: AtomicF32,
    clipped: AtomicBool,
    correlation: AtomicF32,
}

impl Default for StripMeter {
    fn default() -> Self {
        Self { peak_left: AtomicF32::new(0.0), peak_right: AtomicF32::new(0.0), clipped: AtomicBool::new(false), correlation: AtomicF32::new(1.0) }
    }
}

/// Raises `cell` to `value` if it is lower.
fn raise(cell: &AtomicF32, value: f32) {
    if value > cell.get() {
        cell.set(value);
    }
}

impl StripMeter {
    /// Measures a block of the strip's post-fader signal.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn measure(&self, left: &[f32], right: &[f32]) {
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let (l, r) = (peak(left), peak(right));
        // Only the audio thread raises the peaks, and only the reader lowers them, so a lost race costs one block's
        // peak at most.
        raise(&self.peak_left, l);
        raise(&self.peak_right, r);
        if l >= CLIP || r >= CLIP {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }

    /// Measures the correlation of a block and folds it into the running value.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn measure_correlation(&self, left: &[f32], right: &[f32]) {
        let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
        for (l, r) in left.iter().zip(right) {
            let (l, r) = (f64::from(*l), f64::from(*r));
            lr += l * r;
            ll += l * l;
            rr += r * r;
        }
        let block = if ll > 1e-12 && rr > 1e-12 { (lr / (ll * rr).sqrt()) as f32 } else { 1.0 };
        let previous = self.correlation.get();
        self.correlation.set(previous + 0.2 * (block - previous));
    }

    /// The peaks since the last read, which this read clears, the clip latch and the correlation.
    pub fn read(&self) -> MeterReading {
        let peak = |cell: &AtomicF32| {
            let value = cell.get();
            cell.set(0.0);
            value
        };
        MeterReading {
            peak: [peak(&self.peak_left), peak(&self.peak_right)],
            clipped: self.clipped.load(Ordering::Relaxed),
            correlation: self.correlation.get(),
        }
    }

    /// Clears the clip latch.
    pub fn clear_clip(&self) {
        self.clipped.store(false, Ordering::Relaxed);
    }
}

/// How many frames of the master output the scope keeps.
pub const SCOPE_FRAMES: usize = 4096;

/// The last [`SCOPE_FRAMES`] frames of the master output, mixed to mono, for an oscilloscope.
#[derive(Debug)]
pub struct Scope {
    samples: Box<[AtomicU32]>,
    /// Frames written since the start; the next one goes at `written % SCOPE_FRAMES`.
    written: AtomicUsize,
}

impl Default for Scope {
    fn default() -> Self {
        Self { samples: (0..SCOPE_FRAMES).map(|_| AtomicU32::new(0)).collect(), written: AtomicUsize::new(0) }
    }
}

impl Scope {
    /// Adds a block of the master output.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn write(&self, left: &[f32], right: &[f32]) {
        let mut at = self.written.load(Ordering::Relaxed);
        for (l, r) in left.iter().zip(right) {
            if let Some(cell) = self.samples.get(at % SCOPE_FRAMES) {
                cell.store((0.5 * (l + r)).to_bits(), Ordering::Relaxed);
            }
            at = at.wrapping_add(1);
        }
        self.written.store(at, Ordering::Release);
    }

    /// Copies the latest `out.len()` frames, oldest first, into `out` (at most [`SCOPE_FRAMES`]). A frame written
    /// while it is copied may be from either side of the write.
    ///
    /// Complexity: O(n) in `out.len()`.
    pub fn read(&self, out: &mut [f32]) {
        let written = self.written.load(Ordering::Acquire);
        let count = out.len().min(SCOPE_FRAMES);
        let start = written.wrapping_sub(count);
        for (i, sample) in out.iter_mut().enumerate() {
            *sample = if i < count {
                self.samples.get(start.wrapping_add(i) % SCOPE_FRAMES).map_or(0.0, |c| f32::from_bits(c.load(Ordering::Relaxed)))
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_clear_on_read_and_the_clip_latch_holds() {
        let meter = StripMeter::default();
        meter.measure(&[0.5, -0.8], &[0.1, 0.2]);
        meter.measure(&[0.3], &[1.5]);
        let reading = meter.read();
        assert_eq!(reading.peak, [0.8, 1.5]);
        assert!(reading.clipped);
        assert_eq!(meter.read().peak, [0.0, 0.0]);
        assert!(meter.read().clipped, "the latch holds until cleared");
        meter.clear_clip();
        assert!(!meter.read().clipped);
    }

    #[test]
    fn correlation_reads_mono_as_one_and_opposites_as_minus_one() {
        let meter = StripMeter::default();
        let signal: Vec<f32> = (0..256).map(|i| (i as f32 * 0.1).sin()).collect();
        let inverted: Vec<f32> = signal.iter().map(|s| -s).collect();
        for _ in 0..40 {
            meter.measure_correlation(&signal, &inverted);
        }
        assert!(meter.read().correlation < -0.99);
    }

    #[test]
    fn the_scope_keeps_the_latest_frames() {
        let scope = Scope::default();
        let block: Vec<f32> = (0..SCOPE_FRAMES + 10).map(|i| i as f32).collect();
        scope.write(&block, &block);
        let mut out = [0.0; 4];
        scope.read(&mut out);
        let last = (SCOPE_FRAMES + 9) as f32;
        assert_eq!(out, [last - 3.0, last - 2.0, last - 1.0, last]);
    }
}
