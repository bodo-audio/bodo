//! `cargo xtask factory`: synthesizes Bodo's factory content.
//!
//! The Bodo Drums kit is generated here from oscillators, seeded noise, envelopes and filters. Nothing is sampled or
//! copied from anywhere, so the kit is released as CC0-1.0 (`assets/factory/README.md`) and anyone may use it in
//! their music. Every sound is written to `assets/factory/Packs/Bodo Drums/<name>.wav` as 48 kHz mono 24-bit WAV,
//! peak-normalized to -6 dBFS, which leaves headroom for hits that land together, and trimmed where it decays below
//! -90 dBFS.
//!
//! The noise is seeded, so a run on the same machine always writes the same files. Other platforms may round a
//! sine or an exponential differently in the last bit, so the committed files are the reference.

use std::f64::consts::{PI, TAU};
use std::path::Path;

use anyhow::{Context, Result};
use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::SampleRate;
use bodo_kit_dsp::filter::{BiquadCoeffs, BiquadKind};

use crate::workspace;

/// The sample rate of the factory content.
const RATE: f64 = 48_000.0;
/// The peak every sound is normalized to: -6 dBFS, so a kick, an 808 and a hat landing together stay below 0 dBFS.
const PEAK: f64 = 0.501_187_233_627_272_2;
/// Where a sound's tail is cut: -90 dBFS.
const FLOOR: f64 = 3.162_277_660_168_379_5e-5;
/// The fade-in that keeps the first sample from clicking, in seconds.
const FADE_IN: f64 = 0.000_5;
/// The fade-out after the cut, in seconds.
const FADE_OUT: f64 = 0.005;
/// Where the kit is written, relative to the workspace root.
const KIT_DIR: &str = "assets/factory/Packs/Bodo Drums";

/// One sound of the kit: its file name without the extension, and the function that synthesizes it.
struct Sound {
    name: &'static str,
    make: fn() -> Vec<f64>,
}

/// The kit, in the order the files are written.
const KIT: &[Sound] = &[
    Sound { name: "Kick", make: kick },
    Sound { name: "Snare", make: snare },
    Sound { name: "Clap", make: clap },
    Sound { name: "HiHat", make: closed_hat },
    Sound { name: "Open Hat", make: open_hat },
    Sound { name: "Rim", make: rim },
    Sound { name: "Low Tom", make: low_tom },
    Sound { name: "High Tom", make: high_tom },
    Sound { name: "Crash", make: crash },
    Sound { name: "Shaker", make: shaker },
    Sound { name: "808 Bass", make: bass_808 },
];

/// Synthesizes the kit and writes it to [`KIT_DIR`].
pub fn run() -> Result<()> {
    let dir = workspace::root().join(KIT_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    for sound in KIT {
        let samples = render(sound);
        let path = dir.join(format!("{}.wav", sound.name));
        write(&path, &samples)?;
        let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        println!("factory: {:<9} {:.3} s, peak {:.2} dBFS", sound.name, samples.len() as f64 / RATE, 20.0 * f64::from(peak).log10());
    }
    println!("factory: wrote {} sounds to {KIT_DIR}", KIT.len());
    Ok(())
}

/// Synthesizes `sound` and finishes it: DC removed, normalized, trimmed and faded.
fn render(sound: &Sound) -> Vec<f32> {
    finish((sound.make)())
}

fn write(path: &Path, samples: &[f32]) -> Result<()> {
    let rate = SampleRate::HZ_48000;
    let mut writer = WavWriter::create(path, rate, 1, WavFormat::Int24).with_context(|| format!("could not create {}", path.display()))?;
    writer.write(&[samples], samples.len()).with_context(|| format!("could not write {}", path.display()))?;
    writer.finish().with_context(|| format!("could not finish {}", path.display()))
}

/// Removes DC, normalizes the peak to [`PEAK`], cuts the tail below [`FLOOR`] and fades both ends.
fn finish(mut samples: Vec<f64>) -> Vec<f32> {
    dc_block(&mut samples);
    let peak = samples.iter().fold(0.0f64, |peak, s| peak.max(s.abs()));
    if peak > 0.0 {
        for sample in &mut samples {
            *sample *= PEAK / peak;
        }
    }
    let fade_out = seconds(FADE_OUT);
    let last = samples.iter().rposition(|s| s.abs() > FLOOR).unwrap_or(0);
    samples.truncate((last + fade_out).min(samples.len()));
    let len = samples.len();
    let fade_in = seconds(FADE_IN).min(len);
    for (index, sample) in samples.iter_mut().enumerate().take(fade_in) {
        *sample *= raised_cosine(index as f64 / fade_in as f64);
    }
    let fade_out = fade_out.min(len);
    for (index, sample) in samples.iter_mut().skip(len - fade_out).enumerate() {
        *sample *= raised_cosine(1.0 - (index + 1) as f64 / fade_out as f64);
    }
    samples.into_iter().map(|s| s as f32).collect()
}

/// A one-pole high pass at about 8 Hz, which takes out any DC offset the envelopes leave.
fn dc_block(samples: &mut [f64]) {
    let r = 1.0 - TAU * 8.0 / RATE;
    let (mut x1, mut y1) = (0.0, 0.0);
    for sample in samples {
        let y = *sample - x1 + r * y1;
        x1 = *sample;
        y1 = y;
        *sample = y;
    }
}

/// Rises from 0 at `x = 0` to 1 at `x = 1` along half a cosine.
fn raised_cosine(x: f64) -> f64 {
    0.5 - 0.5 * (PI * x.clamp(0.0, 1.0)).cos()
}

/// The number of samples in `duration` seconds.
fn seconds(duration: f64) -> usize {
    (duration * RATE).round() as usize
}

/// A curve that holds 1 until `start` seconds, then falls along half a cosine to 0 at `end`, so a decay reaches
/// silence by `end` without a step.
fn taper(t: f64, start: f64, end: f64) -> f64 {
    if t <= start { 1.0 } else { raised_cosine(1.0 - (t - start) / (end - start)) }
}

/// A short attack, so an envelope never jumps from silence.
fn attack(t: f64, time: f64) -> f64 {
    if t >= time { 1.0 } else { raised_cosine(t / time) }
}

/// Soft saturation that keeps the level of small signals: `tanh(drive * x) / tanh(drive)`.
fn saturate(x: f64, drive: f64) -> f64 {
    (drive * x).tanh() / drive.tanh()
}

/// Seeded white noise (xorshift64*), the same on every run.
struct Noise(u64);

impl Noise {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    /// The next sample, uniform in -1..1.
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let bits = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
        bits as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// A biquad in transposed direct form II, run in `f64`, with coefficients from the Kit's cookbook designs.
struct Filter {
    c: BiquadCoeffs,
    s1: f64,
    s2: f64,
}

impl Filter {
    fn new(kind: BiquadKind, frequency: f64, q: f64) -> Self {
        Self { c: BiquadCoeffs::design(kind, frequency, q, SampleRate::HZ_48000), s1: 0.0, s2: 0.0 }
    }

    fn high_pass(frequency: f64, q: f64) -> Self {
        Self::new(BiquadKind::HighPass, frequency, q)
    }

    fn low_pass(frequency: f64, q: f64) -> Self {
        Self::new(BiquadKind::LowPass, frequency, q)
    }

    fn band_pass(frequency: f64, q: f64) -> Self {
        Self::new(BiquadKind::BandPass, frequency, q)
    }

    fn peak(frequency: f64, q: f64, gain_db: f64) -> Self {
        Self::new(BiquadKind::Peak { gain_db }, frequency, q)
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.c.b0 * x + self.s1;
        self.s1 = self.c.b1 * x - self.c.a1 * y + self.s2;
        self.s2 = self.c.b2 * x - self.c.a2 * y;
        y
    }
}

/// Runs `x` through every filter of `chain` in order.
fn chain(filters: &mut [Filter], x: f64) -> f64 {
    filters.iter_mut().fold(x, |x, filter| filter.process(x))
}

/// A sine whose frequency can change every sample.
#[derive(Default)]
struct Sine {
    phase: f64,
}

impl Sine {
    fn next(&mut self, frequency: f64) -> f64 {
        let out = self.phase.sin();
        self.phase = (self.phase + TAU * frequency / RATE) % TAU;
        out
    }
}

/// A band-limited square (PolyBLEP), so the metallic voices don't alias.
struct Square {
    phase: f64,
    step: f64,
}

impl Square {
    fn new(frequency: f64) -> Self {
        Self { phase: 0.0, step: frequency / RATE }
    }

    fn next(&mut self) -> f64 {
        let (t, dt) = (self.phase, self.step);
        let naive = if t < 0.5 { 1.0 } else { -1.0 };
        let out = naive + blep(t, dt) - blep((t + 0.5) % 1.0, dt);
        self.phase = (t + dt) % 1.0;
        out
    }
}

/// The PolyBLEP correction around a step at phase 0.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// Six detuned squares, the classic recipe for cymbal and hat metal.
struct Metal {
    voices: Vec<Square>,
}

impl Metal {
    fn new(frequencies: &[f64]) -> Self {
        Self { voices: frequencies.iter().map(|&f| Square::new(f)).collect() }
    }

    fn next(&mut self) -> f64 {
        self.voices.iter_mut().map(Square::next).sum::<f64>() / self.voices.len() as f64
    }
}

/// The six oscillator frequencies of the hats.
const HAT_METAL: [f64; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

/// Generates `length` seconds by calling `sample` with each sample's time in seconds.
fn generate(length: f64, mut sample: impl FnMut(f64) -> f64) -> Vec<f64> {
    (0..seconds(length)).map(|index| sample(index as f64 / RATE)).collect()
}

/// A punchy kick: a sine that drops from about 150 Hz to 48 Hz, with a short click on top.
fn kick() -> Vec<f64> {
    let mut body = Sine::default();
    let mut noise = Noise::new(0x6b69_636b);
    let mut click = [Filter::high_pass(1800.0, 0.7), Filter::low_pass(9000.0, 0.7)];
    generate(0.62, |t| {
        // A quick extra sweep from above 150 Hz in the first milliseconds gives the beater's knock.
        let frequency = 48.0 + 102.0 * (-t / 0.045).exp() + 220.0 * (-t / 0.004).exp();
        let amp = attack(t, 0.0008) * (-t / 0.16).exp() * taper(t, 0.42, 0.62);
        let tone = saturate(body.next(frequency) * amp, 1.8);
        let tick = chain(&mut click, noise.next()) * (-t / 0.0025).exp() * 0.5;
        tone + tick
    })
}

/// A snare: two pitched body tones and bright, band-limited noise for the wires.
fn snare() -> Vec<f64> {
    let (mut low, mut high) = (Sine::default(), Sine::default());
    let mut noise = Noise::new(0x736e_6172);
    let mut wires = [Filter::high_pass(1800.0, 0.7), Filter::low_pass(9500.0, 0.6), Filter::peak(4500.0, 0.8, 3.0)];
    let mut snap = [Filter::band_pass(2800.0, 0.9)];
    generate(0.55, |t| {
        let bend = 1.0 + 0.18 * (-t / 0.012).exp();
        let body = 0.75 * low.next(180.0 * bend) * (-t / 0.055).exp() + 0.45 * high.next(330.0 * bend) * (-t / 0.04).exp();
        let n = noise.next();
        let rattle = chain(&mut wires, n) * (-t / 0.11).exp() * taper(t, 0.35, 0.55) * 1.1;
        let crack = chain(&mut snap, n) * (-t / 0.006).exp() * 0.8;
        saturate((body * attack(t, 0.0006) + rattle + crack) * 0.9, 1.4)
    })
}

/// A hand clap: four quick noise bursts, then a band-passed tail.
fn clap() -> Vec<f64> {
    let mut noise = Noise::new(0x636c_6170);
    let mut tone = [Filter::high_pass(700.0, 0.7), Filter::band_pass(1350.0, 1.1), Filter::peak(2400.0, 1.0, 4.0)];
    const BURSTS: [f64; 4] = [0.0, 0.0105, 0.0205, 0.0315];
    generate(0.6, |t| {
        let bursts = BURSTS
            .iter()
            .enumerate()
            .filter(|&(_, &start)| t >= start)
            .map(|(index, &start)| {
                let level = if index + 1 == BURSTS.len() { 1.0 } else { 0.85 };
                level * attack(t - start, 0.0004) * (-(t - start) / 0.0032).exp()
            })
            .fold(0.0f64, f64::max);
        let last = BURSTS[BURSTS.len() - 1];
        let tail = if t >= last { 0.75 * attack(t - last, 0.001) * (-(t - last) / 0.11).exp() * taper(t, 0.4, 0.6) } else { 0.0 };
        chain(&mut tone, noise.next()) * bursts.max(tail) * 2.5
    })
}

/// The shared voice of the hats: metal and a little noise, high-passed around 7 kHz.
fn hat(length: f64, decay: f64, seed: u64) -> Vec<f64> {
    let mut metal = Metal::new(&HAT_METAL.map(|f| f * 1.7));
    let mut noise = Noise::new(seed);
    let mut filters =
        [Filter::high_pass(6800.0, 0.8), Filter::high_pass(6800.0, 0.6), Filter::peak(8800.0, 1.0, 4.0), Filter::low_pass(14_000.0, 0.7)];
    generate(length, |t| {
        let source = metal.next() * 0.8 + noise.next() * 0.35;
        let amp = attack(t, 0.0004) * (-t / decay).exp() * taper(t, length * 0.6, length);
        chain(&mut filters, source) * amp
    })
}

/// A tight closed hat, about 80 ms.
fn closed_hat() -> Vec<f64> {
    hat(0.09, 0.019, 0x6869_6861)
}

/// An open hat that rings for about 600 ms.
fn open_hat() -> Vec<f64> {
    hat(0.62, 0.15, 0x6f70_656e)
}

/// A rim shot: two short, saturated resonances and a stick click.
fn rim() -> Vec<f64> {
    let (mut low, mut high) = (Sine::default(), Sine::default());
    let mut noise = Noise::new(0x7269_6d73);
    let mut stick = [Filter::band_pass(3500.0, 1.2)];
    let mut shape = [Filter::high_pass(320.0, 0.7)];
    generate(0.14, |t| {
        let tone = 0.7 * low.next(470.0) * (-t / 0.009).exp() + 1.0 * high.next(1690.0) * (-t / 0.009).exp();
        let click = chain(&mut stick, noise.next()) * (-t / 0.0025).exp() * 1.4;
        let amp = attack(t, 0.0003) * taper(t, 0.09, 0.14);
        chain(&mut shape, saturate((tone + click) * amp, 3.0))
    })
}

/// A tom: a pitched skin that bends down, a detuned overtone and a stick hit.
fn tom(base: f64, decay: f64, length: f64, seed: u64) -> Vec<f64> {
    let (mut skin, mut overtone) = (Sine::default(), Sine::default());
    let mut noise = Noise::new(seed);
    let mut stick = [Filter::band_pass(base * 14.0, 0.6)];
    generate(length, |t| {
        let bend = 1.0 + 0.45 * (-t / 0.05).exp();
        let body = skin.next(base * bend) * (-t / decay).exp() + 0.3 * overtone.next(base * 1.52 * bend) * (-t / (decay * 0.45)).exp();
        let hit = chain(&mut stick, noise.next()) * (-t / 0.009).exp() * 0.75;
        saturate((body + hit) * attack(t, 0.0008) * taper(t, length * 0.65, length), 1.5)
    })
}

/// A low floor tom.
fn low_tom() -> Vec<f64> {
    tom(96.0, 0.17, 0.75, 0x6c74_6f6d)
}

/// A high rack tom.
fn high_tom() -> Vec<f64> {
    tom(158.0, 0.13, 0.58, 0x6874_6f6d)
}

/// A crash cymbal: dense metal and noise, a bright splash, then a long wash.
fn crash() -> Vec<f64> {
    let mut low_metal = Metal::new(&HAT_METAL.map(|f| f * 1.13));
    let mut high_metal = Metal::new(&HAT_METAL.map(|f| f * 2.71));
    let mut noise = Noise::new(0x6372_6173);
    let mut sizzle =
        [Filter::high_pass(4200.0, 0.7), Filter::high_pass(3000.0, 0.6), Filter::peak(6500.0, 0.9, 4.0), Filter::low_pass(15_000.0, 0.6)];
    let mut body = [Filter::high_pass(900.0, 0.7), Filter::peak(3200.0, 0.8, 4.0), Filter::low_pass(6000.0, 0.7)];
    let mut splash = [Filter::band_pass(7000.0, 0.7)];
    generate(3.2, |t| {
        let n = noise.next();
        let metal = low_metal.next() * high_metal.next();
        let source = metal * 0.7 + n * 0.55;
        let ring = chain(&mut sizzle, source) * (0.65 * (-t / 0.75).exp() + 0.35 * (-t / 0.12).exp());
        let bell = chain(&mut body, source) * (0.6 * (-t / 0.45).exp() + 0.4 * (-t / 0.06).exp()) * 0.9;
        let burst = chain(&mut splash, n) * (-t / 0.03).exp() * 0.6;
        (ring + bell + burst) * attack(t, 0.0015) * taper(t, 2.2, 3.2)
    })
}

/// A shaker: a soft swell of bright noise.
fn shaker() -> Vec<f64> {
    let mut noise = Noise::new(0x7368_616b);
    let mut filters = [Filter::high_pass(5200.0, 0.7), Filter::low_pass(11_500.0, 0.7), Filter::peak(8000.0, 1.0, 3.0)];
    generate(0.17, |t| {
        let swell = if t < 0.014 { (t / 0.014).powi(2) } else { (-(t - 0.014) / 0.035).exp() };
        chain(&mut filters, noise.next()) * swell * taper(t, 0.12, 0.17)
    })
}

/// An 808-style bass on C2: a long sine with a slight drop into pitch and soft saturation for harmonics.
fn bass_808() -> Vec<f64> {
    const C2: f64 = 65.406_391_325_149_66;
    let mut tone = Sine::default();
    generate(1.6, |t| {
        let frequency = C2 * (1.0 + 0.12 * (-t / 0.035).exp());
        let amp = attack(t, 0.002) * (0.82 * (-t / 0.7).exp() + 0.18 * (-t / 0.08).exp()) * taper(t, 1.15, 1.6);
        saturate(tone.next(frequency) * amp, 2.8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbfs(x: f32) -> f64 {
        20.0 * f64::from(x.abs()).log10()
    }

    #[test]
    fn every_sound_is_normalized_trimmed_and_faded() {
        for sound in KIT {
            let samples = render(sound);
            let seconds = samples.len() as f64 / RATE;
            let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            assert!((dbfs(peak) + 6.0).abs() < 0.01, "{}: peak {:.2} dBFS", sound.name, dbfs(peak));
            assert!(seconds > 0.05 && seconds < 3.5, "{}: {seconds:.3} s", sound.name);
            let (first, last) = (samples[0], samples[samples.len() - 1]);
            assert!(dbfs(first) < -80.0 && dbfs(last) < -80.0, "{}: ends at {first} and {last}", sound.name);
            let mean = samples.iter().map(|&s| f64::from(s)).sum::<f64>() / samples.len() as f64;
            assert!(mean.abs() < 0.01, "{}: DC offset {mean}", sound.name);
        }
    }

    #[test]
    fn sounds_are_reproducible() {
        for sound in KIT {
            assert_eq!(render(sound), render(sound), "{}", sound.name);
        }
    }

    #[test]
    fn lengths_match_the_kit() {
        let length = |name: &str| KIT.iter().find(|s| s.name == name).map(|s| render(s).len() as f64 / RATE).unwrap_or(0.0);
        assert!((0.4..=0.65).contains(&length("Kick")), "{}", length("Kick"));
        assert!((0.05..=0.1).contains(&length("HiHat")), "{}", length("HiHat"));
        assert!((0.4..=0.65).contains(&length("Open Hat")), "{}", length("Open Hat"));
        assert!((1.2..=1.65).contains(&length("808 Bass")), "{}", length("808 Bass"));
    }

    #[test]
    fn noise_is_uniform_and_seeded() {
        let mut a = Noise::new(7);
        let values: Vec<f64> = (0..10_000).map(|_| a.next()).collect();
        assert!(values.iter().all(|v| (-1.0..1.0).contains(v)));
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        assert!(mean.abs() < 0.03, "{mean}");
        let mut b = Noise::new(7);
        assert_eq!(b.next(), values[0]);
    }
}
