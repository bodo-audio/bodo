//! Markers stored in WAV files: cue points, regions and sampler loops.
//!
//! Slicing tools write a `cue ` chunk with one point per slice. A region is a cue point with a length, given by a
//! labeled text (`ltxt`) entry in the `LIST`/`adtl` chunk. Loop points live in the `smpl` chunk. All three are read
//! from the RIFF chunks directly, without decoding the audio, and returned in frames of the file's own rate.

use std::path::Path;

use crate::AudioError;

/// The markers of a file, in frames at the file's own rate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleMarkers {
    /// Cue points without a length, sorted and without repeats: the slice boundaries.
    pub cues: Vec<u64>,
    /// Regions, as start and length, sorted by start.
    pub regions: Vec<(u64, u64)>,
    /// Sampler loops, as start and end (exclusive).
    pub loops: Vec<(u64, u64)>,
}

impl SampleMarkers {
    /// Whether the file has no markers at all.
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty() && self.regions.is_empty() && self.loops.is_empty()
    }

    /// The same markers at another rate: every position times `ratio`, rounded.
    ///
    /// Complexity: O(m) in the markers.
    pub fn scaled(&self, ratio: f64) -> Self {
        let at = |frame: u64| (frame as f64 * ratio).round() as u64;
        Self {
            cues: self.cues.iter().map(|c| at(*c)).collect(),
            regions: self.regions.iter().map(|(s, l)| (at(*s), at(*l).max(1))).collect(),
            loops: self.loops.iter().map(|(s, e)| (at(*s), at(*e))).collect(),
        }
    }
}

/// Reads the markers of the WAV file at `path`. A file that isn't a WAV, or has no markers, gives empty markers.
///
/// Does I/O. Complexity: O(file size) to read it, O(m log m) in the markers.
pub fn read_markers(path: &Path) -> Result<SampleMarkers, AudioError> {
    let bytes = std::fs::read(path).map_err(|source| AudioError::Io { path: path.to_path_buf(), source })?;
    Ok(parse_markers(&bytes))
}

/// Parses the markers out of a whole WAV file's bytes.
///
/// Complexity: O(n) in the bytes, O(m log m) in the markers.
pub fn parse_markers(bytes: &[u8]) -> SampleMarkers {
    let mut markers = SampleMarkers::default();
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return markers;
    }
    // Cue point ids and positions, and the lengths `ltxt` entries give some of them.
    let mut points: Vec<(u32, u64)> = Vec::new();
    let mut lengths: Vec<(u32, u64)> = Vec::new();
    for (id, body) in chunks(&bytes[12..]) {
        match id {
            b"cue " => {
                let count = u32_at(body, 0).unwrap_or(0) as usize;
                for i in 0..count {
                    let base = 4 + i * 24;
                    // Each point: id, position, data chunk id, chunk start, block start, sample offset.
                    if let (Some(point), Some(offset)) = (u32_at(body, base), u32_at(body, base + 20)) {
                        points.push((point, u64::from(offset)));
                    }
                }
            }
            b"LIST" if body.get(0..4) == Some(b"adtl") => {
                for (sub, entry) in chunks(&body[4..]) {
                    if sub == b"ltxt"
                        && let (Some(point), Some(length)) = (u32_at(entry, 0), u32_at(entry, 4))
                    {
                        lengths.push((point, u64::from(length)));
                    }
                }
            }
            b"smpl" => {
                let count = u32_at(body, 28).unwrap_or(0) as usize;
                for i in 0..count {
                    // Each loop: id, type, start, end (inclusive), fraction, play count.
                    let base = 36 + i * 24;
                    if let (Some(start), Some(end)) = (u32_at(body, base + 8), u32_at(body, base + 12))
                        && end >= start
                    {
                        markers.loops.push((u64::from(start), u64::from(end) + 1));
                    }
                }
            }
            _ => {}
        }
    }
    for (point, position) in points {
        match lengths.iter().find(|(id, _)| *id == point) {
            Some((_, length)) if *length > 0 => markers.regions.push((position, *length)),
            _ => markers.cues.push(position),
        }
    }
    markers.cues.sort_unstable();
    markers.cues.dedup();
    markers.regions.sort_unstable();
    markers
}

/// The chunks of a RIFF list: four-character id and body, skipping the pad byte after odd sizes.
fn chunks(mut bytes: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let id: &[u8; 4] = bytes.get(0..4)?.try_into().ok()?;
        let size = u32_at(bytes, 4)? as usize;
        let body = bytes.get(8..8 + size).unwrap_or(&bytes[8.min(bytes.len())..]);
        let next = 8 + size + (size & 1);
        bytes = bytes.get(next..).unwrap_or(&[]);
        Some((id, body))
    })
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
}

/// Writes markers into a WAV file's bytes, for tests and for tools that export sliced loops: a `cue ` chunk with every
/// cue and region start, an `adtl` list giving the regions' lengths, and a `smpl` chunk with the loops.
///
/// Complexity: O(n) in the bytes plus the markers.
pub fn with_markers(wav: &[u8], markers: &SampleMarkers) -> Vec<u8> {
    let mut out = wav.to_vec();
    let mut points: Vec<(u64, Option<u64>)> = markers.cues.iter().map(|c| (*c, None)).collect();
    points.extend(markers.regions.iter().map(|(s, l)| (*s, Some(*l))));
    let push_chunk = |out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(id);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
    };
    if !points.is_empty() {
        let mut cue = (points.len() as u32).to_le_bytes().to_vec();
        for (i, (position, _)) in points.iter().enumerate() {
            let position = *position as u32;
            for value in [i as u32 + 1, position] {
                cue.extend_from_slice(&value.to_le_bytes());
            }
            cue.extend_from_slice(b"data");
            for value in [0u32, 0, position] {
                cue.extend_from_slice(&value.to_le_bytes());
            }
        }
        push_chunk(&mut out, b"cue ", &cue);
        let mut list = b"adtl".to_vec();
        for (i, (_, length)) in points.iter().enumerate() {
            if let Some(length) = length {
                let mut ltxt = Vec::new();
                for value in [i as u32 + 1, *length as u32] {
                    ltxt.extend_from_slice(&value.to_le_bytes());
                }
                ltxt.extend_from_slice(b"rgn ");
                ltxt.extend_from_slice(&[0; 8]);
                push_chunk(&mut list, b"ltxt", &ltxt);
            }
        }
        if list.len() > 4 {
            push_chunk(&mut out, b"LIST", &list);
        }
    }
    if !markers.loops.is_empty() {
        let mut smpl = vec![0u8; 28];
        smpl.extend_from_slice(&(markers.loops.len() as u32).to_le_bytes());
        smpl.extend_from_slice(&[0; 4]);
        for (i, (start, end)) in markers.loops.iter().enumerate() {
            for value in [i as u32, 0, *start as u32, end.saturating_sub(1) as u32, 0, 0] {
                smpl.extend_from_slice(&value.to_le_bytes());
            }
        }
        push_chunk(&mut out, b"smpl", &smpl);
    }
    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_wav() -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for i in 0..1000 {
            writer.write_sample(i as i16).unwrap();
        }
        writer.finalize().unwrap();
        cursor.into_inner()
    }

    #[test]
    fn cues_regions_and_loops_round_trip() {
        let markers = SampleMarkers { cues: vec![0, 250, 500, 750], regions: vec![(100, 300), (600, 200)], loops: vec![(200, 800)] };
        let bytes = with_markers(&tiny_wav(), &markers);
        assert_eq!(parse_markers(&bytes), markers);
        // The audio still decodes.
        let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(reader.len(), 1000);
    }

    #[test]
    fn a_plain_wav_or_other_bytes_have_no_markers() {
        assert!(parse_markers(&tiny_wav()).is_empty());
        assert!(parse_markers(b"not audio").is_empty());
    }

    #[test]
    fn scaling_moves_every_marker() {
        let markers = SampleMarkers { cues: vec![100], regions: vec![(10, 20)], loops: vec![(0, 50)] };
        assert_eq!(markers.scaled(2.0), SampleMarkers { cues: vec![200], regions: vec![(20, 40)], loops: vec![(0, 100)] });
    }
}
