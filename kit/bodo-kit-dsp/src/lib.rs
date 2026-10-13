//! DSP building blocks for Bodo Kit.
//!
//! - [`buffer`]: [`AudioBuffer`], planar buffers allocated once and reused per block.
//! - [`gain`]: decibels, gain, ramps and mixing.
//! - [`pan`]: the pan laws for mono sources, and stereo-fold panning.
//! - [`stereo`]: stereo separation, polarity and channel swap.
//! - [`smooth`]: parameter smoothing.
//! - [`delay`]: exact integer delay lines.
//! - [`filter`]: RBJ biquads and a state-variable filter.
//! - [`envelope`]: the ADSR envelope.
//! - [`interp`]: interpolation between samples.
//!
//! Samples are `f32`. Everything that runs per block is RT-safe and marked `#[rt_safe]`.

#![forbid(unsafe_code)]

pub mod buffer;
pub mod delay;
pub mod envelope;
pub mod filter;
pub mod gain;
pub mod interp;
pub mod pan;
pub mod smooth;
pub mod stereo;

pub use buffer::AudioBuffer;
