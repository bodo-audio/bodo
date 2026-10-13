//! Loading the project's samples into memory at the project's sample rate (project.md section 6.7).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_audio::{AudioData, decode_file, resample};
use bodo_kit_core::SampleRate;
use bodo_model::{Project, SampleId};

/// Decoded samples, shared by every sampler that plays them.
#[derive(Debug, Default)]
pub struct SampleCache {
    loaded: HashMap<(PathBuf, SampleRate), Arc<AudioData>>,
}

impl SampleCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The sample `id` of `project`, decoded and converted to the project's rate. Paths are relative to `base`.
    /// Returns `None`, after logging why, when the sample is missing or can't be read.
    ///
    /// Does I/O the first time a file is asked for.
    pub fn get(&mut self, project: &Project, id: SampleId, base: &Path) -> Option<Arc<AudioData>> {
        let reference = project.samples.get(&id)?;
        let path = base.join(&reference.path);
        let rate = project.settings.sample_rate;
        if let Some(data) = self.loaded.get(&(path.clone(), rate)) {
            return Some(Arc::clone(data));
        }
        let data = match decode_file(&path).and_then(|audio| resample(audio, rate)) {
            Ok(data) => Arc::new(data),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "could not load a sample");
                return None;
            }
        };
        self.loaded.insert((path, rate), Arc::clone(&data));
        Some(data)
    }

    /// Forgets samples nothing uses any more.
    pub fn trim(&mut self) {
        self.loaded.retain(|_, data| Arc::strong_count(data) > 1);
    }
}
