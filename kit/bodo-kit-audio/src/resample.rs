//! Converting audio to another sample rate when it is loaded (project.md section 6.7).
//!
//! The rate pair is fixed, so this uses Rubato's synchronous FFT resampler, which is fast and has a flat passband.
//! It runs off the audio thread, once per file.
//!
//! The output can be shifted by a constant fraction of a sample (about half a sample from 44.1 to 48 kHz, roughly
//! 10 microseconds). The shift is the same for every file with the same rate pair, so files stay aligned with each
//! other, and it is far below anything audible or any timing tolerance in the project.

use audioadapter_buffers::direct::SequentialSliceOfVecs;
use bodo_kit_core::SampleRate;
use rubato::{Fft, FixedSync, Resampler};

use crate::{AudioData, AudioError};

/// Returns `audio` converted to `rate`. Audio already at `rate` is returned unchanged.
///
/// Allocates. Complexity: O(n log c) in the frames, with c the FFT chunk size.
pub fn resample(audio: AudioData, rate: SampleRate) -> Result<AudioData, AudioError> {
    if audio.rate == rate || audio.frames() == 0 {
        return Ok(AudioData { rate, ..audio });
    }
    let channels = audio.channels.len();
    let frames = audio.frames();
    let mut resampler = Fft::<f32>::new(audio.rate.hz() as usize, rate.hz() as usize, 1024, channels, FixedSync::Both)
        .map_err(|err| AudioError::Resample(err.to_string()))?;
    let input = SequentialSliceOfVecs::new(&audio.channels, channels, frames).map_err(|err| AudioError::Resample(err.to_string()))?;
    let needed = resampler.process_all_needed_output_len(frames);
    let mut output: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.0; needed]).collect();
    let mut adapter = SequentialSliceOfVecs::new_mut(&mut output, channels, needed).map_err(|err| AudioError::Resample(err.to_string()))?;
    let (_, produced) = resampler.process_all_into_buffer(&input, &mut adapter, frames, None).map_err(|err| AudioError::Resample(err.to_string()))?;
    for channel in &mut output {
        channel.truncate(produced);
    }
    Ok(AudioData { rate, channels: output })
}
