//! A peaks pyramid for drawing waveforms at any zoom (project.md section 6.7).
//!
//! Level 0 stores the minimum and maximum of every 32 frames; each level above combines 4 entries of the one below.
//! Any range query then touches at most a few entries per level, O(log n), however long the range is.

/// Frames summarized by each level-0 entry.
pub const BASE: usize = 32;
/// Entries of one level combined into one entry of the next.
pub const FAN_OUT: usize = 4;

/// The peaks pyramid of one channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelPeaks {
    /// `levels[k]` holds `(min, max)` pairs, each covering `BASE * FAN_OUT^k` frames.
    levels: Vec<Vec<(f32, f32)>>,
    frames: usize,
}

impl ChannelPeaks {
    /// Builds the pyramid of `samples`.
    ///
    /// Allocates. Complexity: O(n) in the number of samples.
    pub fn build(samples: &[f32]) -> Self {
        let base: Vec<(f32, f32)> = samples.chunks(BASE).map(min_max).collect();
        let mut levels = vec![base];
        while let Some(last) = levels.last()
            && last.len() > 1
        {
            let next = last.chunks(FAN_OUT).map(combine).collect();
            levels.push(next);
        }
        Self { levels, frames: samples.len() }
    }

    /// The number of frames summarized.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The minimum and maximum over frames `start..end`, widened to whole 32-frame blocks at the ends. `None` for an
    /// empty range.
    ///
    /// Complexity: O(log n) in the number of frames.
    pub fn range(&self, start: usize, end: usize) -> Option<(f32, f32)> {
        let end = end.min(self.frames);
        if start >= end {
            return None;
        }
        let (mut lo, mut hi) = (start / BASE, end.div_ceil(BASE));
        let mut result: Option<(f32, f32)> = None;
        let mut take = |pair: (f32, f32)| result = Some(result.map_or(pair, |r| (r.0.min(pair.0), r.1.max(pair.1))));
        for (level, entries) in self.levels.iter().enumerate() {
            if lo >= hi {
                break;
            }
            if level + 1 == self.levels.len() {
                entries.get(lo..hi.min(entries.len())).into_iter().flatten().for_each(|&p| take(p));
                break;
            }
            while lo < hi && lo % FAN_OUT != 0 {
                if let Some(&p) = entries.get(lo) {
                    take(p);
                }
                lo += 1;
            }
            while lo < hi && hi % FAN_OUT != 0 {
                hi -= 1;
                if let Some(&p) = entries.get(hi) {
                    take(p);
                }
            }
            lo /= FAN_OUT;
            hi /= FAN_OUT;
        }
        result
    }

    /// Fills `out` with one `(min, max)` per pixel, for pixels of `frames_per_pixel` frames starting at `start`.
    /// Pixels past the end are `(0.0, 0.0)`.
    ///
    /// Complexity: O(p log n) in the number of pixels and frames.
    pub fn pixels(&self, start: usize, frames_per_pixel: usize, out: &mut [(f32, f32)]) {
        let step = frames_per_pixel.max(1);
        for (i, pixel) in out.iter_mut().enumerate() {
            let from = start + i * step;
            *pixel = self.range(from, from + step).unwrap_or((0.0, 0.0));
        }
    }
}

fn min_max(samples: &[f32]) -> (f32, f32) {
    samples.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &s| (lo.min(s), hi.max(s)))
}

fn combine(pairs: &[(f32, f32)]) -> (f32, f32) {
    pairs.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_shrink_by_the_fan_out() {
        let peaks = ChannelPeaks::build(&vec![0.0; BASE * 64]);
        let sizes: Vec<usize> = peaks.levels.iter().map(Vec::len).collect();
        assert_eq!(sizes, [64, 16, 4, 1]);
    }

    #[test]
    fn range_finds_the_extremes() {
        let mut samples = vec![0.0f32; 10_000];
        samples[5_000] = 0.9;
        samples[7_777] = -0.4;
        let peaks = ChannelPeaks::build(&samples);
        assert_eq!(peaks.range(0, 10_000), Some((-0.4, 0.9)));
        assert_eq!(peaks.range(0, 4_992), Some((0.0, 0.0)));
        assert_eq!(peaks.range(4_992, 5_024), Some((0.0, 0.9)));
        assert_eq!(peaks.range(10, 10), None);
        assert_eq!(peaks.range(9_990, 20_000), Some((0.0, 0.0)));
    }
}
