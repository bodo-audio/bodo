//! Planar audio buffers, allocated once at their largest size and then reused for every block.

use bodo_kit_core::{MAX_BLOCK_FRAMES, rt_safe};

/// A buffer of `channels` planar channels with room for up to `capacity` frames. Its current length can change
/// per block without reallocating.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    data: Vec<f32>,
    channels: usize,
    capacity: usize,
    frames: usize,
}

impl AudioBuffer {
    /// A silent buffer with `channels` channels and room for `capacity` frames, initially `capacity` frames long.
    ///
    /// Allocates. Complexity: O(channels * capacity).
    pub fn new(channels: usize, capacity: usize) -> Self {
        Self { data: vec![0.0; channels * capacity], channels, capacity, frames: capacity }
    }

    /// A silent buffer sized for the largest block, [`MAX_BLOCK_FRAMES`].
    ///
    /// Allocates. Complexity: O(channels * MAX_BLOCK_FRAMES).
    pub fn for_blocks(channels: usize) -> Self {
        Self::new(channels, MAX_BLOCK_FRAMES)
    }

    /// The number of channels.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The current length in frames.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The largest length this buffer can take.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Sets the current length, clamped to the capacity, and returns the length set. The samples are left as they
    /// were.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn set_frames(&mut self, frames: usize) -> usize {
        self.frames = frames.min(self.capacity);
        self.frames
    }

    /// Channel `index`, or an empty slice when there is no such channel.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn channel(&self, index: usize) -> &[f32] {
        let start = index * self.capacity;
        self.data.get(start..start + self.frames).unwrap_or(&[])
    }

    /// Channel `index`, mutably, or an empty slice when there is no such channel.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn channel_mut(&mut self, index: usize) -> &mut [f32] {
        let start = index * self.capacity;
        let frames = self.frames;
        self.data.get_mut(start..start + frames).unwrap_or(&mut [])
    }

    /// Every channel, mutably, in order.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn channels_mut(&mut self) -> impl Iterator<Item = &mut [f32]> {
        let frames = self.frames;
        self.data.chunks_mut(self.capacity.max(1)).take(self.channels).map(move |channel| &mut channel[..frames])
    }

    /// The first two channels, mutably, or `None` when the buffer has fewer than two.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn stereo_mut(&mut self) -> Option<(&mut [f32], &mut [f32])> {
        if self.channels < 2 {
            return None;
        }
        let frames = self.frames;
        let (left, rest) = self.data.split_at_mut(self.capacity);
        Some((&mut left[..frames], &mut rest[..frames]))
    }

    /// Sets the current frames of every channel to silence.
    ///
    /// RT-safe. Complexity: O(channels * frames).
    #[rt_safe]
    pub fn clear(&mut self) {
        for channel in self.channels_mut() {
            channel.fill(0.0);
        }
    }

    /// Copies the current frames of `other` into this buffer, channel by channel, over the channels and frames both
    /// have.
    ///
    /// RT-safe. Complexity: O(channels * frames).
    #[rt_safe]
    pub fn copy_from(&mut self, other: &AudioBuffer) {
        for (index, channel) in self.channels_mut().enumerate() {
            let source = other.channel(index);
            let frames = channel.len().min(source.len());
            channel[..frames].copy_from_slice(&source[..frames]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_planar_and_follow_the_length() {
        let mut buffer = AudioBuffer::new(2, 8);
        assert_eq!(buffer.set_frames(4), 4);
        buffer.channel_mut(0).fill(1.0);
        buffer.channel_mut(1).fill(2.0);
        assert_eq!(buffer.channel(0), [1.0; 4]);
        assert_eq!(buffer.channel(1), [2.0; 4]);
        assert_eq!(buffer.channel(2), [] as [f32; 0]);
        assert_eq!(buffer.set_frames(100), 8);
        assert_eq!(buffer.channel(0)[4..], [0.0; 4]);
    }

    #[test]
    fn stereo_halves_and_copy() {
        let mut a = AudioBuffer::new(2, 4);
        let (l, r) = a.stereo_mut().unwrap();
        l.fill(0.5);
        r.fill(-0.5);
        let mut b = AudioBuffer::new(2, 4);
        b.copy_from(&a);
        assert_eq!(b, a);
        b.clear();
        assert!(b.channels_mut().all(|c| c.iter().all(|&s| s == 0.0)));
        assert!(AudioBuffer::new(1, 4).stereo_mut().is_none());
    }
}
