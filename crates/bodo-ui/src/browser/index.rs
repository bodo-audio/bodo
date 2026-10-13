//! The Browser's index of the library, built on a background thread so the interface never waits for the disk.

use std::sync::Arc;
use std::sync::mpsc;

use bodo_engine::Env;
use bodo_engine::library::UserLibrary;
use bodo_engine::library::index::LibraryIndex;
use bodo_engine::library::tags::TagStore;

/// The index, or the thread building it.
#[derive(Debug, Default)]
pub(super) enum IndexState {
    /// Not built yet, or dropped by a refresh.
    #[default]
    Empty,
    /// Being built for this environment.
    Building(mpsc::Receiver<LibraryIndex>, Env),
    /// Built.
    Ready(Arc<LibraryIndex>, Env),
}

impl IndexState {
    /// The index for `env`, once built. Starts building it when there is none, or when `env` changed since.
    pub(super) fn get(&mut self, env: &Env) -> Option<Arc<LibraryIndex>> {
        let stale = match self {
            Self::Empty => true,
            Self::Building(_, built) | Self::Ready(_, built) => built != env,
        };
        if stale {
            let (tx, rx) = mpsc::channel();
            let for_thread = env.clone();
            let spawned = std::thread::Builder::new().name(String::from("bodo-library-index")).spawn(move || {
                let _ = tx.send(LibraryIndex::build(&for_thread));
            });
            *self = match spawned {
                Ok(_) => Self::Building(rx, env.clone()),
                Err(error) => {
                    tracing::warn!(%error, "could not start reading the library");
                    Self::Ready(Arc::new(LibraryIndex::default()), env.clone())
                }
            };
        }
        if let Self::Building(rx, built) = self {
            match rx.try_recv() {
                Ok(index) => *self = Self::Ready(Arc::new(index), built.clone()),
                Err(mpsc::TryRecvError::Disconnected) => *self = Self::Ready(Arc::new(LibraryIndex::default()), built.clone()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        match self {
            Self::Ready(index, _) => Some(Arc::clone(index)),
            _ => None,
        }
    }

    /// Whether the index is being built.
    pub(super) fn is_building(&self) -> bool {
        matches!(self, Self::Building(..))
    }

    /// Takes the user's tags again from `library`, after they changed.
    pub(super) fn retag(&mut self, library: &UserLibrary) {
        if let Self::Ready(index, _) = self {
            Arc::make_mut(index).apply(&TagStore::load(library));
        }
    }
}
