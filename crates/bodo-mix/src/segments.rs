//! The pieces of audio clips a block plays, which the sequencer works out and audio clip nodes render.
//!
//! A segment says where in the block a clip plays, where in its audio to read and how fast, and where it is inside
//! the clip for fades. Nodes keep no position of their own, so seeking, stopping and looping need nothing special:
//! every block describes itself.

use bodo_kit_core::rt_safe;

/// The most audio clip segments one block can carry. More are dropped and counted.
pub const MAX_SEGMENTS: usize = 1024;

/// A stretch of one audio clip inside one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSegment {
    /// The first frame of the block it covers.
    pub frame: u32,
    /// How many frames it covers.
    pub frames: u32,
    /// Where to read at `frame`, in frames of the audio it reads.
    pub position: f64,
    /// Frames of that audio per output frame.
    pub step: f64,
    /// The key of the time-stretched audio to read, or `None` for the sample itself.
    pub stretched: Option<u32>,
    /// The stretch ratio, to fall back on reading the sample faster when the stretched audio isn't ready.
    pub ratio: f64,
    /// Output frames since the clip's start, at `frame`.
    pub clip_position: f64,
    /// The clip's length in output frames.
    pub clip_frames: f64,
    /// The fade-in's length in output frames; zero for none.
    pub fade_in: f64,
    /// The fade-out's length in output frames; zero for none.
    pub fade_out: f64,
}

impl AudioSegment {
    /// The fade gain `offset` frames into the segment: 0 to 1.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn fade(&self, offset: usize) -> f32 {
        let at = self.clip_position + offset as f64;
        let mut gain = 1.0f64;
        if self.fade_in > 0.0 {
            gain = gain.min(at / self.fade_in);
        }
        if self.fade_out > 0.0 {
            gain = gain.min((self.clip_frames - at) / self.fade_out);
        }
        gain.clamp(0.0, 1.0) as f32
    }
}

/// The segments of one block, grouped by channel.
#[derive(Debug)]
pub struct Segments {
    items: Vec<(u64, AudioSegment)>,
    /// One entry per channel with segments: its key and its range in `items`. Sorted by key.
    directory: Vec<(u64, u32, u32)>,
    /// Segments dropped because a block was full, since the engine started.
    pub dropped: u64,
}

impl Default for Segments {
    fn default() -> Self {
        Self::new()
    }
}

impl Segments {
    /// An empty list with room for [`MAX_SEGMENTS`] segments.
    ///
    /// Allocates.
    pub fn new() -> Self {
        Self { items: Vec::with_capacity(MAX_SEGMENTS), directory: Vec::with_capacity(MAX_SEGMENTS), dropped: 0 }
    }

    /// Clears the list for a new block.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn clear(&mut self) {
        self.items.clear();
        self.directory.clear();
    }

    /// Adds a segment for the channel with `key`. When the block is full the segment is dropped and counted.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn push(&mut self, key: u64, segment: AudioSegment) {
        if self.items.len() < MAX_SEGMENTS {
            self.items.push((key, segment));
        } else {
            self.dropped += 1;
        }
    }

    /// Groups the segments by channel. Call once after the last push of a block.
    ///
    /// RT-safe. Complexity: O(s log s) in the number of segments.
    #[rt_safe]
    pub fn finish(&mut self) {
        self.items.sort_unstable_by_key(|(key, segment)| (*key, segment.frame));
        let mut start = 0;
        while start < self.items.len() {
            let key = self.items[start].0;
            let end = start + self.items[start..].iter().take_while(|(k, _)| *k == key).count();
            self.directory.push((key, start as u32, end as u32));
            start = end;
        }
    }

    /// The segments of the channel with `key`, in block order.
    ///
    /// RT-safe. Complexity: O(log c) in the channels with segments this block.
    #[rt_safe]
    pub fn for_key(&self, key: u64) -> impl Iterator<Item = &AudioSegment> {
        let range = match self.directory.binary_search_by_key(&key, |&(k, _, _)| k) {
            Ok(index) => self.directory.get(index).map_or(0..0, |&(_, start, end)| start as usize..end as usize),
            Err(_) => 0..0,
        };
        self.items.get(range).unwrap_or(&[]).iter().map(|(_, segment)| segment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(frame: u32) -> AudioSegment {
        AudioSegment {
            frame,
            frames: 4,
            position: 0.0,
            step: 1.0,
            stretched: None,
            ratio: 1.0,
            clip_position: 0.0,
            clip_frames: 100.0,
            fade_in: 10.0,
            fade_out: 20.0,
        }
    }

    #[test]
    fn segments_group_by_channel_in_block_order() {
        let mut segments = Segments::new();
        segments.push(7, segment(40));
        segments.push(3, segment(0));
        segments.push(7, segment(10));
        segments.finish();
        let seven: Vec<u32> = segments.for_key(7).map(|s| s.frame).collect();
        assert_eq!(seven, [10, 40]);
        assert_eq!(segments.for_key(3).count(), 1);
        assert_eq!(segments.for_key(1).count(), 0);
    }

    #[test]
    fn fades_ramp_in_and_out() {
        let s = segment(0);
        assert_eq!(s.fade(0), 0.0);
        assert!((s.fade(5) - 0.5).abs() < 1e-6);
        assert_eq!(s.fade(50), 1.0);
        let late = AudioSegment { clip_position: 90.0, ..s };
        assert!((late.fade(0) - 0.5).abs() < 1e-6, "10 frames before the end of a 20-frame fade-out");
    }
}
