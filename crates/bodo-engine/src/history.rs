//! Undo and redo (project.md section 8.4).
//!
//! A step is a label and the snapshot from before it. Snapshots share everything they didn't change, so undo and redo
//! swap pointers. A gesture, such as a knob drag, folds into one step: a commit with the same merge key as the newest
//! step keeps that step's "before" and only moves the project forward.

use std::collections::VecDeque;
use std::sync::Arc;

use bodo_model::Project;

/// How much history to keep. The oldest steps go first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryLimits {
    /// The most undo steps.
    pub max_steps: usize,
    /// Roughly how many bytes of notes and clips only the history keeps alive.
    pub max_bytes: usize,
}

impl Default for HistoryLimits {
    /// 500 steps or 256 MB, whichever comes first.
    fn default() -> Self {
        Self { max_steps: 500, max_bytes: 256 << 20 }
    }
}

#[derive(Clone, Debug)]
struct Step {
    label: String,
    project: Arc<Project>,
    merge_key: Option<String>,
    bytes: usize,
}

/// The undo and redo stacks.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    bytes: usize,
    limits: HistoryLimits,
}

impl History {
    /// Records a step from `before` to `after`. Returns false when it merged into the newest step instead.
    ///
    /// Complexity: O(p + t) in patterns and tracks, to estimate what the step keeps alive.
    pub(crate) fn commit(&mut self, label: String, before: Arc<Project>, after: &Project, merge_key: Option<String>) -> bool {
        self.redo.clear();
        if let (Some(key), Some(top)) = (&merge_key, self.undo.back_mut())
            && top.merge_key.as_ref() == Some(key)
        {
            top.label = label;
            return false;
        }
        let bytes = unshared_bytes(&before, after);
        self.bytes += bytes;
        self.undo.push_back(Step { label, project: before, merge_key, bytes });
        self.enforce();
        true
    }

    /// Moves back one step from `current`, returning the project to restore and the step's label.
    pub(crate) fn undo(&mut self, current: Arc<Project>) -> Option<(Arc<Project>, String)> {
        let step = self.undo.pop_back()?;
        self.bytes = self.bytes.saturating_sub(step.bytes);
        let restored = Arc::clone(&step.project);
        self.redo.push(Step { label: step.label.clone(), project: current, merge_key: None, bytes: 0 });
        Some((restored, step.label))
    }

    /// Moves forward one undone step from `current`.
    pub(crate) fn redo(&mut self, current: Arc<Project>) -> Option<(Arc<Project>, String)> {
        let step = self.redo.pop()?;
        let restored = Arc::clone(&step.project);
        let bytes = unshared_bytes(&current, &restored);
        self.bytes += bytes;
        self.undo.push_back(Step { label: step.label.clone(), project: current, merge_key: None, bytes });
        Some((restored, step.label))
    }

    /// Ends any gesture in progress, so the next commit starts a new step even with the same merge key.
    pub(crate) fn seal(&mut self) {
        if let Some(top) = self.undo.back_mut() {
            top.merge_key = None;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
    }

    pub(crate) fn set_limits(&mut self, limits: HistoryLimits) {
        self.limits = limits;
        self.enforce();
    }

    /// Labels of the steps that can be undone, oldest first.
    pub(crate) fn undo_labels(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().map(|s| s.label.as_str())
    }

    /// Labels of the steps that can be redone, next first.
    pub(crate) fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|s| s.label.as_str())
    }

    /// Applies `f` to every snapshot the history holds, for example to rebase sample paths after a save.
    ///
    /// Complexity: O(h) calls of `f`.
    pub(crate) fn rewrite(&mut self, mut f: impl FnMut(&Project) -> Option<Project>) {
        for step in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if let Some(project) = f(&step.project) {
                step.project = Arc::new(project);
            }
        }
    }

    fn enforce(&mut self) {
        while self.undo.len() > self.limits.max_steps || (self.bytes > self.limits.max_bytes && self.undo.len() > 1) {
            match self.undo.pop_front() {
                Some(step) => self.bytes = self.bytes.saturating_sub(step.bytes),
                None => break,
            }
        }
    }
}

/// Roughly the bytes of notes and clips in `old` that `new` doesn't share.
///
/// Complexity: O(p + t) in patterns and tracks; nothing when both maps are shared.
fn unshared_bytes(old: &Project, new: &Project) -> usize {
    const BASE: usize = 512;
    let mut bytes = BASE;
    if !old.patterns.ptr_eq(&new.patterns) {
        for (id, pattern) in &old.patterns {
            if !new.patterns.get(id).is_some_and(|p| Arc::ptr_eq(p, pattern)) {
                let notes: usize = pattern.notes.values().map(|seq| seq.len()).sum();
                bytes += BASE + notes * size_of::<bodo_model::Note>();
            }
        }
    }
    if !old.tracks.ptr_eq(&new.tracks) {
        for (id, track) in &old.tracks {
            if !new.tracks.get(id).is_some_and(|t| Arc::ptr_eq(t, track)) {
                bytes += BASE + track.clips.len() * size_of::<bodo_model::Clip>();
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tempo: f64) -> Arc<Project> {
        Arc::new(Project::new(tempo).unwrap())
    }

    #[test]
    fn steps_undo_redo_and_merge() {
        let mut history = History::default();
        let (a, b, c, d) = (project(100.0), project(110.0), project(120.0), project(130.0));
        assert!(history.commit("one".into(), Arc::clone(&a), &b, None));
        assert!(history.commit("drag".into(), Arc::clone(&b), &c, Some("k#1".into())));
        assert!(!history.commit("drag".into(), Arc::clone(&c), &d, Some("k#1".into())), "same key merges");
        assert_eq!(history.undo_labels().collect::<Vec<_>>(), ["one", "drag"]);
        let (back, label) = history.undo(Arc::clone(&d)).unwrap();
        assert!(Arc::ptr_eq(&back, &b), "the merged step undoes the whole gesture");
        assert_eq!(label, "drag");
        let (forward, _) = history.redo(back).unwrap();
        assert!(Arc::ptr_eq(&forward, &d));
        history.seal();
        assert!(history.commit("drag".into(), Arc::clone(&d), &a, Some("k#1".into())), "a sealed step never merges");
    }

    #[test]
    fn limits_drop_the_oldest_steps() {
        let mut history = History::default();
        history.set_limits(HistoryLimits { max_steps: 2, max_bytes: usize::MAX });
        for (n, label) in ["a", "b", "c"].into_iter().enumerate() {
            history.commit(label.into(), project(100.0 + n as f64), &project(200.0), None);
        }
        assert_eq!(history.undo_labels().collect::<Vec<_>>(), ["b", "c"]);
    }
}
