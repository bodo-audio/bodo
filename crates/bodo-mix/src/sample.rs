//! Loading the project's samples into memory at the project's sample rate (project.md section 6.7).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_audio::{AudioData, Decoder, SampleMarkers, decode_file, read_markers, resample};
use bodo_kit_core::SampleRate;
use bodo_model::{Project, SampleId, SampleRoots};

/// Decoded samples, shared by every sampler that plays them.
#[derive(Debug, Default)]
pub struct SampleCache {
    loaded: HashMap<(PathBuf, SampleRate), Arc<AudioData>>,
    /// Samples at their own rate, for samplers that don't resample.
    raw: HashMap<PathBuf, Arc<AudioData>>,
    /// Each file's markers and its own rate, when it has markers.
    markers: HashMap<PathBuf, (SampleMarkers, Option<SampleRate>)>,
    /// Files that could not be loaded, so they aren't read again on every update.
    failed: HashSet<(PathBuf, SampleRate)>,
    /// Files that could not be loaded at their own rate.
    failed_raw: HashSet<PathBuf>,
}

impl SampleCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The sample `id` of `project`, decoded and converted to the project's rate. Stored paths resolve through `roots`.
    /// Returns `None`, after logging why, when the sample is missing or can't be read; it doesn't try that file again
    /// until [`SampleCache::retry_failed`].
    ///
    /// Does I/O the first time a file is asked for.
    pub fn get(&mut self, project: &Project, id: SampleId, roots: &SampleRoots) -> Option<Arc<AudioData>> {
        let reference = project.samples.get(&id)?;
        let path = roots.resolve(&reference.path);
        let rate = project.settings.sample_rate;
        let key = (path, rate);
        if let Some(data) = self.loaded.get(&key) {
            return Some(Arc::clone(data));
        }
        if self.failed.contains(&key) {
            return None;
        }
        let path = &key.0;
        let data = match decode_file(path).and_then(|audio| resample(audio, rate)) {
            Ok(data) => Arc::new(data),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "could not load a sample");
                self.failed.insert(key);
                return None;
            }
        };
        self.loaded.insert(key, Arc::clone(&data));
        Some(data)
    }

    /// The sample `id` of `project` at the file's own rate, for a sampler that reads it faster or slower instead of
    /// converting it. Like [`SampleCache::get`] otherwise.
    ///
    /// Does I/O the first time a file is asked for.
    pub fn get_raw(&mut self, project: &Project, id: SampleId, roots: &SampleRoots) -> Option<Arc<AudioData>> {
        let reference = project.samples.get(&id)?;
        let path = roots.resolve(&reference.path);
        if let Some(data) = self.raw.get(&path) {
            return Some(Arc::clone(data));
        }
        if self.failed_raw.contains(&path) {
            return None;
        }
        match decode_file(&path) {
            Ok(data) => {
                let data = Arc::new(data);
                self.raw.insert(path, Arc::clone(&data));
                Some(data)
            }
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "could not load a sample");
                self.failed_raw.insert(path);
                None
            }
        }
    }

    /// The markers of sample `id` (cue points, regions and loops), in frames at `rate`. A file without markers, or
    /// that can't be read, has none.
    ///
    /// Does I/O the first time a file is asked for. Complexity: O(m) in the markers after that.
    pub fn markers(&mut self, project: &Project, id: SampleId, roots: &SampleRoots, rate: SampleRate) -> SampleMarkers {
        let Some(reference) = project.samples.get(&id) else { return SampleMarkers::default() };
        let path = roots.resolve(&reference.path);
        let (markers, own) = self.markers.entry(path).or_insert_with_key(|path| load_markers(path));
        match own {
            Some(own) if *own != rate && !markers.is_empty() => markers.scaled(f64::from(rate.hz()) / f64::from(own.hz())),
            _ => markers.clone(),
        }
    }

    /// Tries files that failed to load again on their next use, for example after the user found a missing file.
    pub fn retry_failed(&mut self) {
        self.failed.clear();
        self.failed_raw.clear();
        self.markers.clear();
    }

    /// Forgets samples nothing uses any more.
    pub fn trim(&mut self) {
        self.loaded.retain(|_, data| Arc::strong_count(data) > 1);
        self.raw.retain(|_, data| Arc::strong_count(data) > 1);
    }
}

/// A file's markers and its own rate; nothing when it has none or can't be read.
fn load_markers(path: &Path) -> (SampleMarkers, Option<SampleRate>) {
    let markers = read_markers(path).unwrap_or_default();
    if markers.is_empty() {
        return (markers, None);
    }
    let rate = Decoder::open(path).ok().map(|d| d.info().rate);
    (markers, rate)
}
