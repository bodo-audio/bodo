//! Loading the project's samples into memory at the project's sample rate (project.md section 6.7).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use bodo_kit_audio::{AudioData, decode_file, resample};
use bodo_kit_core::SampleRate;
use bodo_model::{Project, SampleId, SampleRoots};

/// Decoded samples, shared by every sampler that plays them.
#[derive(Debug, Default)]
pub struct SampleCache {
    loaded: HashMap<(PathBuf, SampleRate), Arc<AudioData>>,
    /// Files that could not be loaded, so they aren't read again on every update.
    failed: HashSet<(PathBuf, SampleRate)>,
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

    /// Tries files that failed to load again on their next use, for example after the user found a missing file.
    pub fn retry_failed(&mut self) {
        self.failed.clear();
    }

    /// Forgets samples nothing uses any more.
    pub fn trim(&mut self) {
        self.loaded.retain(|_, data| Arc::strong_count(data) > 1);
    }
}
