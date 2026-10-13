//! The ADSR envelope used by the sampler and synths.
//!
//! Attack rises linearly from the current level to full. Decay and release fall exponentially, reaching -60 dB of
//! the distance to their goal after the set time, which sounds natural and never steps. Once the release has fallen
//! below -100 dB the envelope is idle, and the voice using it can be freed.

use bodo_kit_core::{SampleRate, Seconds, rt_safe};

/// Envelope settings.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub struct AdsrParams {
    /// Time to rise to full level.
    pub attack: Seconds,
    /// Time to fall from full level to the sustain level.
    pub decay: Seconds,
    /// Level held while the note is held, 0 to 1.
    pub sustain: f32,
    /// Time to fall to silence after the note is released.
    pub release: Seconds,
}

impl Default for AdsrParams {
    fn default() -> Self {
        Self { attack: Seconds(0.002), decay: Seconds(0.1), sustain: 1.0, release: Seconds(0.05) }
    }
}

/// Where the envelope is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Silent, waiting for a note.
    #[default]
    Idle,
    /// Rising.
    Attack,
    /// Falling toward the sustain level.
    Decay,
    /// Holding at the sustain level.
    Sustain,
    /// Falling to silence.
    Release,
}

/// Distance to the goal, relative to the start, after a decay or release time: -60 dB.
const TIME_RATIO: f64 = 1e-3;
/// Level below which a release counts as finished: -100 dB.
const SILENT: f32 = 1e-5;

/// An ADSR envelope generator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adsr {
    stage: Stage,
    level: f32,
    attack_step: f32,
    decay_coef: f32,
    release_coef: f32,
    sustain: f32,
}

impl Adsr {
    /// An idle envelope with `params` at `rate`.
    pub fn new(params: AdsrParams, rate: SampleRate) -> Self {
        let mut env = Self { stage: Stage::Idle, level: 0.0, attack_step: 1.0, decay_coef: 0.0, release_coef: 0.0, sustain: 1.0 };
        env.set_params(params, rate);
        env
    }

    /// Changes the settings; a note in progress continues with the new ones.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn set_params(&mut self, params: AdsrParams, rate: SampleRate) {
        let samples = |time: Seconds| time.0.max(0.0) * f64::from(rate.hz());
        let attack = samples(params.attack);
        self.attack_step = if attack < 1.0 { 1.0 } else { (1.0 / attack) as f32 };
        let coef = |time: Seconds| {
            let n = samples(time);
            if n < 1.0 { 0.0 } else { TIME_RATIO.powf(1.0 / n) as f32 }
        };
        self.decay_coef = coef(params.decay);
        self.release_coef = coef(params.release);
        self.sustain = params.sustain.clamp(0.0, 1.0);
    }

    /// Starts the attack from the current level, so retriggering doesn't click.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Starts the release from the current level.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// Silences the envelope at once.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
    }

    /// Advances one sample and returns the level.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn next_level(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += self.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if (self.level - self.sustain).abs() < SILENT {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = self.sustain,
            Stage::Release => {
                self.level *= self.release_coef;
                if self.level < SILENT {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }

    /// Multiplies `samples` by the envelope, advancing it one step per sample.
    ///
    /// RT-safe. Complexity: O(n) in the number of samples.
    #[rt_safe]
    pub fn apply(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample *= self.next_level();
        }
    }

    /// The current stage.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The current level.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Whether the envelope is silent and waiting for a note.
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: SampleRate = SampleRate::HZ_48000;

    fn params(attack: f64, decay: f64, sustain: f32, release: f64) -> AdsrParams {
        AdsrParams { attack: Seconds(attack), decay: Seconds(decay), sustain, release: Seconds(release) }
    }

    #[test]
    fn walks_through_every_stage() {
        let mut env = Adsr::new(params(0.001, 0.01, 0.5, 0.01), RATE);
        assert!(env.is_idle());
        env.gate_on();
        let after_attack: Vec<f32> = (0..48).map(|_| env.next_level()).collect();
        assert!((after_attack[47] - 1.0).abs() < 1e-5);
        assert!((after_attack[23] - 0.5).abs() < 0.02, "attack is linear");
        for _ in 0..4800 {
            env.next_level();
        }
        assert_eq!(env.stage(), Stage::Sustain);
        assert_eq!(env.level(), 0.5);
        env.gate_off();
        for _ in 0..2400 {
            env.next_level();
        }
        assert!(env.is_idle());
        assert_eq!(env.level(), 0.0);
    }

    #[test]
    fn release_reaches_minus_sixty_db_after_its_time() {
        let mut env = Adsr::new(params(0.0, 0.0, 1.0, 0.1), RATE);
        env.gate_on();
        env.next_level();
        env.next_level();
        env.gate_off();
        let level = (0..4800).map(|_| env.next_level()).last().unwrap();
        assert!((20.0 * level.log10() + 60.0).abs() < 0.1, "{level}");
    }

    #[test]
    fn retrigger_starts_from_the_current_level() {
        let mut env = Adsr::new(params(0.01, 0.0, 1.0, 1.0), RATE);
        env.gate_on();
        for _ in 0..240 {
            env.next_level();
        }
        env.gate_off();
        let before = env.next_level();
        env.gate_on();
        let after = env.next_level();
        assert!(after > before && after - before < 0.01, "no jump: {before} -> {after}");
    }
}
