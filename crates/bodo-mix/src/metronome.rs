//! The metronome and count-in (project.md section 14.1).
//!
//! Clicks are synthesized once, off the audio thread: a higher click on the first beat of each bar, a lower one on the
//! other beats. While the transport plays with the metronome on, a click starts on the exact frame of every beat,
//! found through the tempo and meter maps the same way the sequencer places notes. Count-in clicks a bar at the tempo
//! and meter of the record position before the playhead moves. Clicks are mixed straight into the output, after the
//! master.

use bodo_kit_core::{PPQ, SampleRate, Seconds, Tick, rt_safe};
use bodo_kit_dsp::AudioBuffer;
use bodo_model::meter::MeterMap;

use crate::sequence::SongIndex;

/// The most clicks that can start in one block. Beats are at least 60 ms apart even at 1000 BPM, and blocks are at
/// most 4096 frames, so a few suffice.
const MAX_CLICKS: usize = 8;

/// A click's length in seconds.
const CLICK_SECONDS: f64 = 0.04;

/// The click sound and the click that is still ringing.
#[derive(Debug)]
pub(crate) struct Metronome {
    high: Vec<f32>,
    low: Vec<f32>,
    /// The ringing click: whether it is the high one, and how far it has played.
    ringing: Option<(bool, usize)>,
}

/// One click of the size `len` at `frequency` Hz and `gain`, decaying fast after a 1 ms attack.
fn click(rate: SampleRate, frequency: f64, gain: f32) -> Vec<f32> {
    let hz = f64::from(rate.hz());
    let len = (CLICK_SECONDS * hz) as usize;
    (0..len)
        .map(|i| {
            let t = i as f64 / hz;
            let attack = (t / 0.001).min(1.0);
            let envelope = attack * (-t * 90.0).exp();
            ((std::f64::consts::TAU * frequency * t).sin() * envelope) as f32 * gain
        })
        .collect()
}

/// Where a block's clicks start, and which are high.
#[derive(Debug, Default)]
struct Starts {
    at: [(usize, bool); MAX_CLICKS],
    len: usize,
}

impl Starts {
    fn push(&mut self, offset: usize, high: bool) {
        if let Some(slot) = self.at.get_mut(self.len) {
            *slot = (offset, high);
            self.len += 1;
        }
    }
}

impl Metronome {
    /// The clicks at `rate`.
    ///
    /// Allocates.
    pub(crate) fn new(rate: SampleRate) -> Self {
        Self { high: click(rate, 1760.0, 0.5), low: click(rate, 1320.0, 0.35), ringing: None }
    }

    /// Stops the ringing click, for a stop or a jump.
    pub(crate) fn silence(&mut self) {
        self.ringing = None;
    }

    /// Adds the clicks of the song block that starts at timeline frame `first` and lasts `frames` frames into
    /// `out[at..at + frames]`.
    ///
    /// RT-safe. Complexity: O(frames) plus O(log m) per beat in the block.
    #[rt_safe]
    pub(crate) fn song(&mut self, song: &SongIndex, rate: SampleRate, first: i64, frames: usize, out: &mut AudioBuffer, at: usize) {
        let hz = f64::from(rate.hz());
        let end = first + frames as i64;
        let tempo = song.tempo();
        let meter = song.meter();
        let t0 = tempo.ticks_at(Seconds((first - 1) as f64 / hz)).floor().max(0.0) as i64;
        let t1 = tempo.ticks_at(Seconds((end + 1) as f64 / hz)).ceil() as i64;
        let mut starts = Starts::default();
        let mut tick = next_beat(meter, t0);
        while tick <= t1 {
            let frame = (tempo.seconds_at(Tick(tick)).0 * hz).round() as i64;
            if frame >= first && frame < end {
                starts.push((frame - first) as usize, meter.position(Tick(tick)).beat == 0);
            }
            tick = next_beat(meter, tick + 1);
        }
        self.mix(&starts, frames, out, at);
    }

    /// Adds the count-in clicks for `frames` frames into `out[at..at + frames]`, `elapsed` frames into a count-in whose
    /// beats are `beat` frames apart.
    ///
    /// RT-safe. Complexity: O(frames).
    #[rt_safe]
    pub(crate) fn count_in(&mut self, elapsed: i64, beat: i64, frames: usize, out: &mut AudioBuffer, at: usize) {
        let beat = beat.max(1);
        let mut starts = Starts::default();
        let mut next = (elapsed + beat - 1) / beat * beat;
        while next < elapsed + frames as i64 {
            starts.push((next - elapsed) as usize, next == 0);
            next += beat;
        }
        self.mix(&starts, frames, out, at);
    }

    fn mix(&mut self, starts: &Starts, frames: usize, out: &mut AudioBuffer, at: usize) {
        let channels = out.channels();
        for frame in 0..frames {
            if let Some(&(_, high)) = starts.at[..starts.len].iter().find(|(offset, _)| *offset == frame) {
                self.ringing = Some((high, 0));
            }
            let Some((high, position)) = self.ringing else { continue };
            let sound = if high { &self.high } else { &self.low };
            let Some(&sample) = sound.get(position) else {
                self.ringing = None;
                continue;
            };
            for channel in 0..channels {
                if let Some(value) = out.channel_mut(channel).get_mut(at + frame) {
                    *value += sample;
                }
            }
            self.ringing = Some((high, position + 1));
        }
    }
}

/// The first beat at or after `tick`.
///
/// Complexity: O(log m) in meter changes.
pub(crate) fn next_beat(meter: &MeterMap, tick: i64) -> i64 {
    let into = meter.position(Tick(tick)).tick;
    if into == 0 { tick } else { tick + meter.signature_at(Tick(tick)).beat_ticks() - into }
}

/// The frames of one beat and the beats of one bar at `tick`, for a count-in.
///
/// Complexity: O(log n) in tempo and meter changes.
pub(crate) fn count_in_beats(song: &SongIndex, rate: SampleRate, tick: Tick) -> (i64, u8) {
    let signature = song.meter().signature_at(tick);
    let beat_ticks = signature.beat_ticks();
    let bpm = song.tempo().bpm_at(tick);
    let seconds = beat_ticks as f64 / PPQ as f64 * 60.0 / bpm;
    ((seconds * f64::from(rate.hz())).round() as i64, signature.numerator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bodo_model::meter::MeterChange;

    #[test]
    fn beats_follow_the_meter() {
        let meter =
            MeterMap::new(vec![MeterChange { bar: 0, numerator: 4, denominator: 4 }, MeterChange { bar: 1, numerator: 6, denominator: 8 }]).unwrap();
        assert_eq!(next_beat(&meter, 0), 0);
        assert_eq!(next_beat(&meter, 1), 960);
        assert_eq!(next_beat(&meter, 3000), 3840);
        assert_eq!(next_beat(&meter, 3841), 3840 + 480, "eighth-note beats in 6/8");
    }
}
