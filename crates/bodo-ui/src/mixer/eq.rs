//! The strip EQ in the insert panel (project.md section 9.2): its response curve and a frequency, gain and Q knob for
//! each of the three bands, the low shelf, the peaking mid and the high shelf. The curve is the one the engine plays:
//! the same biquads, designed the same way, at the project's sample rate.

use bodo_kit_core::SampleRate;
use bodo_kit_dsp::filter::{BiquadCoeffs, BiquadKind};
use bodo_kit_ui::Knob;
use bodo_model::mixer::{EqBand, StripEq};
use bodo_model::{MixerInsert, Project};
use egui::{Align2, FontId, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::insert_ref;
use crate::i18n::{en, t};
use crate::theme::{self, Tokens, rem, size};
use crate::{BodoApp, control};

/// The lowest and highest frequencies the knobs reach and the curve shows.
const LOW_HZ: f32 = 20.0;
const HIGH_HZ: f32 = 20_000.0;
/// The gain range of a band, in dB.
const GAIN_DB: f32 = 24.0;
/// The Q range of a band.
const LOW_Q: f32 = 0.1;
const HIGH_Q: f32 = 10.0;
/// Points along the curve.
const POINTS: usize = 96;

/// A frequency's place on a knob or the curve, 0 to 1, spread logarithmically.
fn frequency_position(hz: f32) -> f32 {
    ((hz.clamp(LOW_HZ, HIGH_HZ) / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln()).clamp(0.0, 1.0)
}

/// The frequency at position `p`, 0 to 1.
fn frequency_at(p: f32) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf(p.clamp(0.0, 1.0))
}

/// A Q's place on a knob, 0 to 1, spread logarithmically.
fn q_position(q: f32) -> f32 {
    ((q.clamp(LOW_Q, HIGH_Q) / LOW_Q).ln() / (HIGH_Q / LOW_Q).ln()).clamp(0.0, 1.0)
}

/// The Q at position `p`, 0 to 1.
fn q_at(p: f32) -> f32 {
    LOW_Q * (HIGH_Q / LOW_Q).powf(p.clamp(0.0, 1.0))
}

fn format_frequency(p: f32) -> String {
    let hz = frequency_at(p);
    if hz >= 1000.0 { format!("{:.2} kHz", hz / 1000.0) } else { format!("{hz:.0} Hz") }
}

fn format_q(p: f32) -> String {
    format!("Q {:.2}", q_at(p))
}

fn format_gain(db: f32) -> String {
    format!("{db:+.1} dB")
}

/// The EQ's gain at `hz`, in dB: the three bands' responses added up.
///
/// Complexity: O(1).
pub(crate) fn response_db(eq: &StripEq, hz: f64, rate: SampleRate) -> f64 {
    let bands = [
        (BiquadKind::LowShelf { gain_db: f64::from(eq.low.gain_db) }, eq.low),
        (BiquadKind::Peak { gain_db: f64::from(eq.mid.gain_db) }, eq.mid),
        (BiquadKind::HighShelf { gain_db: f64::from(eq.high.gain_db) }, eq.high),
    ];
    bands
        .iter()
        .filter(|(_, band)| band.gain_db != 0.0)
        .map(|(kind, band)| {
            let coeffs = BiquadCoeffs::design(*kind, f64::from(band.frequency.max(1.0)), f64::from(band.q.max(0.05)), rate);
            20.0 * coeffs.magnitude(hz, rate).max(1e-9).log10()
        })
        .sum()
}

/// Draws the EQ: its curve and the knobs of its three bands.
pub(super) fn show(app: &mut BodoApp, ui: &mut Ui, project: &Project, insert: &MixerInsert, number: usize) {
    let tk = Tokens::get();
    ui.label(RichText::new(t("EQ")).size(size::SMALL).color(tk.text_dim));
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, rem(5.12)), Sense::hover());
    control::record(ui, &response, "Panel EQ curve", "graph", None);
    app.hint(&response, "The EQ's response: the low shelf, the peaking mid and the high shelf together");
    curve(ui, rect, &insert.eq, project.settings.sample_rate);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = rem(0.16);
        for (band, name, eq_band) in [("low", en("Low"), insert.eq.low), ("mid", en("Mid"), insert.eq.mid), ("high", en("High"), insert.eq.high)] {
            ui.vertical(|ui| {
                ui.label(RichText::new(t(name)).size(size::SMALL).color(tk.text_faint));
                band_knobs(app, ui, number, band, name, eq_band);
            });
            ui.add_space(rem(0.32));
        }
    });
}

/// The frequency, gain and Q knobs of one band; `name` is the band's English name.
fn band_knobs(app: &mut BodoApp, ui: &mut Ui, number: usize, band: &str, name: &'static str, values: EqBand) {
    let insert = insert_ref(number);
    let label = |what: &str| format!("{} EQ {name} {what}", super::insert_label(number));
    let shown = t(name);
    let diameter = rem(1.6);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let mut p = frequency_position(values.frequency);
        let knob = ui.add(Knob::new(&mut p, 0.0..=1.0).diameter(diameter).format(format_frequency).label(tf!("{band} band frequency", band = shown)));
        control::record(ui, &knob, &label("frequency"), "knob", None);
        app.hint(&knob, tf!("{band} band frequency: {value}", band = shown, value = format_frequency(p)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_eq", json!({ "insert": insert, "band": band, "frequency": frequency_at(p) }));
        }
        let mut gain = values.gain_db;
        let knob = ui.add(
            Knob::new(&mut gain, -GAIN_DB..=GAIN_DB)
                .origin(0.0)
                .default(0.0)
                .diameter(diameter)
                .format(format_gain)
                .label(tf!("{band} band gain", band = shown)),
        );
        control::record(ui, &knob, &label("gain"), "knob", None);
        app.hint(&knob, tf!("{band} band gain: {value}", band = shown, value = format_gain(gain)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_eq", json!({ "insert": insert, "band": band, "gainDb": gain }));
        }
        let mut q = q_position(values.q);
        let knob = ui.add(Knob::new(&mut q, 0.0..=1.0).diameter(diameter).format(format_q).label(tf!("{band} band Q", band = shown)));
        control::record(ui, &knob, &label("Q"), "knob", None);
        app.hint(&knob, tf!("{band} band {value} (for the shelves, the slope)", band = shown, value = format_q(q)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_eq", json!({ "insert": insert, "band": band, "q": q_at(q) }));
        }
    });
}

/// The response curve, from 20 Hz to 20 kHz and -24 to +24 dB, with lines at 100 Hz, 1 kHz and 10 kHz and at 0 dB.
fn curve(ui: &Ui, rect: Rect, eq: &StripEq, rate: SampleRate) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 3.0, tk.field);
    let y_of = |db: f64| rect.center().y - (db as f32 / GAIN_DB) * (rect.height() / 2.0 - rem(0.24));
    for hz in [100.0, 1000.0, 10_000.0] {
        let x = rect.left() + frequency_position(hz) * rect.width();
        painter.vline(x, rect.y_range(), Stroke::new(1.0, tk.grid_beat));
        let text = if hz >= 1000.0 { format!("{}k", hz / 1000.0) } else { format!("{hz}") };
        painter.text(pos2(x + rem(0.16), rect.bottom() - rem(0.16)), Align2::LEFT_BOTTOM, text, FontId::monospace(rem(0.64)), tk.text_faint);
    }
    painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, tk.grid_bar));
    let points: Vec<egui::Pos2> = (0..=POINTS)
        .map(|i| {
            let p = i as f32 / POINTS as f32;
            let hz = f64::from(frequency_at(p)).min(f64::from(rate.hz()) * 0.49);
            pos2(rect.left() + p * rect.width(), y_of(response_db(eq, hz, rate).clamp(-f64::from(GAIN_DB), f64::from(GAIN_DB))))
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.5, tk.accent)));
    for band in [eq.low, eq.mid, eq.high] {
        let x = rect.left() + frequency_position(band.frequency) * rect.width();
        let y = y_of(f64::from(band.gain_db));
        painter.circle_filled(pos2(x, y), rem(0.24), theme::faded(tk.accent, if band.gain_db == 0.0 { 0.4 } else { 1.0 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_eq_reads_zero_and_a_boost_reads_its_gain_at_the_center() {
        let rate = SampleRate::HZ_48000;
        let flat = StripEq::default();
        assert!(response_db(&flat, 1000.0, rate).abs() < 1e-9);
        let boosted = StripEq { mid: EqBand { frequency: 1000.0, gain_db: 6.0, q: 1.0 }, ..StripEq::default() };
        assert!((response_db(&boosted, 1000.0, rate) - 6.0).abs() < 0.05);
        assert!(response_db(&boosted, 50.0, rate).abs() < 0.5, "far from the band, little changes");
    }

    #[test]
    fn knob_positions_round_trip() {
        for hz in [20.0, 120.0, 1200.0, 8000.0, 20_000.0] {
            assert!((frequency_at(frequency_position(hz)) - hz).abs() / hz < 1e-4);
        }
        for q in [0.1, 0.707, 1.4, 10.0] {
            assert!((q_at(q_position(q)) - q).abs() / q < 1e-4);
        }
    }
}
