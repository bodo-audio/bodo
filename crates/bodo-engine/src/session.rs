//! The editing session: the current project, its history, the editor state and the transport (ADR 0005).
//!
//! [`Session::execute`] is the one path every change takes. An edit runs on a scratch copy of the project in a
//! [`Ctx`]; when it succeeds the session checks the invariants (in debug builds and tests), commits the copy as one
//! undo step, and keeps the events for the frontend. When it fails nothing changes. A batch is one transaction and one
//! undo step.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_core::Tick;
use bodo_model::mixer::RouteSource;
use bodo_model::{Channel, ChannelId, ClipId, InsertId, MixerInsert, Pattern, PatternId, PlaylistTrack, Project, SampleRoots, TrackId};
use serde_json::Value;

use crate::error::{CommandError, ErrorCode};
use crate::history::{History, HistoryLimits};
use crate::ids::IdSource;
use crate::params::{ChannelRef, ClipRef, InsertRef, PatternRef, RouteFromRef, TrackRef};
use crate::registry::{Effects, Handler, Registry};
use crate::resolve;
use crate::state::{EditorState, Event, TransportState};
use crate::template::{Template, new_project};

/// Where the session finds content outside the project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// Bodo's factory content, where `factory:` sample paths start.
    pub factory: Option<PathBuf>,
    /// Folders of the user's own sounds, which the browser lists.
    pub library: Vec<PathBuf>,
    /// The user library, where presets, templates, recordings and exports go ([`crate::library::user`]); `None` when
    /// there is none, as in tests.
    pub user: Option<PathBuf>,
}

/// One command to run.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    /// The command id, such as `channel.add`.
    pub id: String,
    /// The params; empty params act on the current selection.
    #[serde(default)]
    pub params: Value,
    /// The gesture this call belongs to, such as one knob drag. Calls of one gesture fold into one undo step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<u64>,
}

impl Call {
    /// A call of `id` with `params`.
    pub fn new(id: impl Into<String>, params: Value) -> Self {
        Self { id: id.into(), params, gesture: None }
    }

    /// The same call as part of gesture `gesture`.
    pub fn in_gesture(self, gesture: u64) -> Self {
        Self { gesture: Some(gesture), ..self }
    }
}

/// What a command did.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    /// The command id.
    pub id: String,
    /// The command's own result.
    pub result: Value,
    /// The label of the undo step it made or extended, when it changed the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
    /// Whether the project changed.
    pub changed: bool,
    /// The project's revision afterwards.
    pub revision: u64,
}

/// What a batch did.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOutput {
    /// Each command's result, in order.
    pub results: Vec<Value>,
    /// The label of the one undo step the batch made, when it changed the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
    /// Whether the project changed.
    pub changed: bool,
    /// The project's revision afterwards.
    pub revision: u64,
}

/// Why a batch didn't run. Nothing in it took effect.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, thiserror::Error)]
#[error("command {index} ({id}) failed, so the batch did nothing: {error}")]
pub struct BatchError {
    /// The position of the failing command, from 0; the batch's length when the commit itself failed.
    pub index: usize,
    /// Its id.
    pub id: String,
    /// What went wrong.
    pub error: CommandError,
}

/// What an edit produced, once it no longer borrows the session.
struct Outcome {
    project: Project,
    editor: EditorState,
    transport: TransportState,
    events: Vec<Event>,
}

/// What an edit works on: scratch copies of the project, the editor state and the transport, plus the session's
/// environment. Nothing reaches the session unless the edit succeeds.
pub struct Ctx<'a> {
    /// The project, a scratch copy. Copying it costs a handful of pointers.
    pub project: Project,
    /// The editor state, a scratch copy.
    pub editor: EditorState,
    /// The transport, a scratch copy.
    pub transport: TransportState,
    env: &'a Env,
    file: Option<&'a Path>,
    ids: &'a mut IdSource,
    events: Vec<Event>,
    label: Option<String>,
}

impl std::fmt::Debug for Ctx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx").field("editor", &self.editor).field("transport", &self.transport).finish_non_exhaustive()
    }
}

impl Ctx<'_> {
    /// The session's environment.
    pub fn env(&self) -> &Env {
        self.env
    }

    /// The project's file, if it was saved or opened.
    pub fn file(&self) -> Option<&Path> {
        self.file
    }

    /// The folder relative sample paths start from: the one that holds the project's file, if it has one.
    pub fn project_dir(&self) -> Option<PathBuf> {
        self.file.map(bodo_io::project_folder)
    }

    /// Where stored sample paths point.
    pub fn sample_roots(&self) -> SampleRoots {
        roots(self.file, self.env)
    }

    /// Where new ids come from.
    pub fn ids(&mut self) -> &mut IdSource {
        self.ids
    }

    /// Queues an event for the frontend, sent only if the command succeeds.
    pub fn emit(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Sets the label of the undo step, such as "Set Kick volume".
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = Some(label.into());
    }

    /// The channel `r` names, or else the channel the piano roll edits, or else the one selected channel.
    ///
    /// Complexity: O(c) in channels.
    pub fn channel(&self, r: Option<&ChannelRef>) -> Result<ChannelId, CommandError> {
        match r {
            Some(r) => resolve::channel(&self.project, r),
            None => self
                .editor
                .channel
                .or_else(|| if self.editor.channels.len() == 1 { self.editor.channels.first().copied() } else { None })
                .ok_or_else(|| CommandError::nothing_selected("channel")),
        }
    }

    /// The channels `refs` name, or else the selected channels.
    ///
    /// Complexity: O(r * c) in references and channels.
    pub fn channels(&self, refs: &[ChannelRef]) -> Result<Vec<ChannelId>, CommandError> {
        if refs.is_empty() {
            let selected: Vec<ChannelId> = self.editor.channels.iter().copied().collect();
            return if selected.is_empty() { Err(CommandError::nothing_selected("channels")) } else { Ok(selected) };
        }
        let mut ids = Vec::with_capacity(refs.len());
        for r in refs {
            let id = resolve::channel(&self.project, r)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// The pattern `r` names, or else the current pattern.
    ///
    /// Complexity: O(p) in patterns.
    pub fn pattern(&self, r: Option<&PatternRef>) -> Result<PatternId, CommandError> {
        match r {
            Some(r) => resolve::pattern(&self.project, r),
            None => self.editor.pattern.filter(|p| self.project.patterns.contains_key(p)).ok_or_else(|| CommandError::nothing_selected("pattern")),
        }
    }

    /// The track `r` names, or else the selected track, or else the first track.
    ///
    /// Complexity: O(t) in tracks.
    pub fn track(&self, r: Option<&TrackRef>) -> Result<TrackId, CommandError> {
        match r {
            Some(r) => resolve::track(&self.project, r),
            None => self
                .editor
                .track
                .filter(|t| self.project.tracks.contains_key(t))
                .or_else(|| self.project.track_order.front().copied())
                .ok_or_else(|| CommandError::nothing_selected("track").with_hint("add a track with playlist.add_track")),
        }
    }

    /// The insert `r` names, or else the selected insert.
    ///
    /// Complexity: O(i) in inserts.
    pub fn insert(&self, r: Option<&InsertRef>) -> Result<InsertId, CommandError> {
        match r {
            Some(r) => resolve::insert(&self.project, r),
            None => self.editor.insert.filter(|i| self.project.mixer.insert(*i).is_some()).ok_or_else(|| CommandError::nothing_selected("insert")),
        }
    }

    /// The channel or insert a route starts from.
    ///
    /// Complexity: O(c + i) in channels and inserts.
    pub fn route_source(&self, r: &RouteFromRef) -> Result<RouteSource, CommandError> {
        resolve::route_source(&self.project, r)
    }

    /// The clips `refs` name with their tracks, or else the selected clips.
    ///
    /// Complexity: O(r * n) in references and clips on the playlist.
    pub fn clips(&self, refs: &[ClipRef]) -> Result<Vec<(TrackId, ClipId)>, CommandError> {
        if refs.is_empty() {
            let found: Vec<(TrackId, ClipId)> = self
                .project
                .tracks
                .values()
                .flat_map(|t| t.clips.iter().filter(|c| self.editor.clips.contains(&c.id)).map(move |c| (t.id, c.id)))
                .collect();
            return if found.is_empty() { Err(CommandError::nothing_selected("clips")) } else { Ok(found) };
        }
        let mut found = Vec::with_capacity(refs.len());
        for r in refs {
            let clip = resolve::clip(&self.project, r)?;
            if !found.contains(&clip) {
                found.push(clip);
            }
        }
        Ok(found)
    }

    /// Changes channel `id` with `f`, storing it only when something changed.
    pub fn edit_channel(&mut self, id: ChannelId, f: impl FnOnce(&mut Channel)) -> Result<(), CommandError> {
        let old = self.project.channels.get(&id).ok_or_else(|| CommandError::not_found("channel", &id.to_string()))?;
        let mut new = (**old).clone();
        f(&mut new);
        if new != **old {
            self.project.channels.insert(id, Arc::new(new));
        }
        Ok(())
    }

    /// Changes pattern `id` with `f`, which returns whether it changed anything and a result.
    pub fn edit_pattern<T>(&mut self, id: PatternId, f: impl FnOnce(&mut Pattern) -> Result<(bool, T), CommandError>) -> Result<T, CommandError> {
        let old = self.project.patterns.get(&id).ok_or_else(|| CommandError::not_found("pattern", &id.to_string()))?;
        let mut new = (**old).clone();
        let (changed, result) = f(&mut new)?;
        if changed {
            self.project.patterns.insert(id, Arc::new(new));
        }
        Ok(result)
    }

    /// Changes track `id` with `f`, storing it only when something changed.
    pub fn edit_track<T>(&mut self, id: TrackId, f: impl FnOnce(&mut PlaylistTrack) -> Result<T, CommandError>) -> Result<T, CommandError> {
        let old = self.project.tracks.get(&id).ok_or_else(|| CommandError::not_found("track", &id.to_string()))?;
        let mut new = (**old).clone();
        let result = f(&mut new)?;
        if new != **old {
            self.project.tracks.insert(id, Arc::new(new));
        }
        Ok(result)
    }

    /// Changes insert `id` (the master included) with `f`, storing it only when something changed.
    pub fn edit_insert(&mut self, id: InsertId, f: impl FnOnce(&mut MixerInsert)) -> Result<(), CommandError> {
        let old = self.project.mixer.insert(id).ok_or_else(|| CommandError::not_found("insert", &id.to_string()))?;
        let mut new = (**old).clone();
        f(&mut new);
        if new == **old {
            return Ok(());
        }
        let mixer = Arc::make_mut(&mut self.project.mixer);
        if mixer.master.id == id {
            mixer.master = Arc::new(new);
        } else {
            mixer.inserts.insert(id, Arc::new(new));
        }
        Ok(())
    }

    fn finish(self) -> (Outcome, Option<String>) {
        (Outcome { project: self.project, editor: self.editor, transport: self.transport, events: self.events }, self.label)
    }
}

/// The sample roots of a project saved at `file`.
fn roots(file: Option<&Path>, env: &Env) -> SampleRoots {
    let project = file.map(bodo_io::project_folder).or_else(|| std::env::current_dir().ok()).unwrap_or_default();
    SampleRoots::new(project).with_factory(env.factory.clone())
}

/// Whether two vectors hold the same items. Comparing pointers is enough for large vectors; imbl keeps tiny ones inline,
/// where pointers never match, but comparing those by value is cheap.
fn same_vector<T: Clone + PartialEq>(a: &imbl::Vector<T>, b: &imbl::Vector<T>) -> bool {
    a.ptr_eq(b) || a == b
}

/// Whether two snapshots share every part, so nothing changed between them.
fn same_snapshot(a: &Project, b: &Project) -> bool {
    a.id == b.id
        && Arc::ptr_eq(&a.meta, &b.meta)
        && Arc::ptr_eq(&a.settings, &b.settings)
        && Arc::ptr_eq(&a.tempo, &b.tempo)
        && Arc::ptr_eq(&a.meter, &b.meter)
        && a.channels.ptr_eq(&b.channels)
        && same_vector(&a.rack_order, &b.rack_order)
        && a.patterns.ptr_eq(&b.patterns)
        && a.tracks.ptr_eq(&b.tracks)
        && same_vector(&a.track_order, &b.track_order)
        && Arc::ptr_eq(&a.mixer, &b.mixer)
        && a.samples.ptr_eq(&b.samples)
        && same_vector(&a.markers, &b.markers)
}

/// The current project and everything that changes it. Single-owner and synchronous: frontends call it from one
/// thread, or send their requests to the thread that owns it.
pub struct Session {
    registry: Arc<Registry>,
    project: Arc<Project>,
    /// The project as last saved or opened, to tell whether there are unsaved changes.
    saved: Option<Arc<Project>>,
    file: Option<PathBuf>,
    history: History,
    editor: EditorState,
    transport: TransportState,
    env: Env,
    ids: IdSource,
    events: Vec<Event>,
    revision: u64,
    validate: bool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("file", &self.file).field("revision", &self.revision).finish_non_exhaustive()
    }
}

impl Session {
    /// A session with a new project from the basic template: 16 playlist tracks, one pattern, the master and 16
    /// inserts, and a starter drum kit when the factory content is available.
    pub fn new(env: Env) -> Self {
        Self::build(env, IdSource::random(), false)
    }

    /// A session whose ids count up from a fixed start and whose projects carry no creation time, so tests and
    /// snapshots are the same on every run. It checks the invariants after every command.
    pub fn deterministic(env: Env) -> Self {
        Self::build(env, IdSource::counting(0x1000), true)
    }

    fn build(env: Env, mut ids: IdSource, validate: bool) -> Self {
        let created = (!ids.is_counting()).then(crate::template::now_rfc3339);
        let (project, editor) = new_project(Template::Basic, &mut ids, &env, created);
        let project = Arc::new(project);
        Self {
            registry: Arc::new(Registry::builtin()),
            saved: Some(Arc::clone(&project)),
            project,
            file: None,
            history: History::default(),
            editor,
            transport: TransportState::default(),
            env,
            ids,
            events: Vec::new(),
            revision: 0,
            validate: validate || cfg!(debug_assertions),
        }
    }

    /// The command registry.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// The current project.
    pub fn project(&self) -> &Arc<Project> {
        &self.project
    }

    /// The editor's current pattern, channel and selection.
    pub fn editor(&self) -> &EditorState {
        &self.editor
    }

    /// The transport.
    pub fn transport(&self) -> &TransportState {
        &self.transport
    }

    /// The environment.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Changes the environment, for example after the user added a sample folder.
    pub fn set_env(&mut self, env: Env) {
        self.env = env;
    }

    /// The project's file, if it was saved or opened.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Where the project's stored sample paths point.
    pub fn sample_roots(&self) -> SampleRoots {
        roots(self.file.as_deref(), &self.env)
    }

    /// Whether the project changed since it was last saved or opened.
    pub fn is_dirty(&self) -> bool {
        !self.saved.as_ref().is_some_and(|saved| Arc::ptr_eq(saved, &self.project))
    }

    /// A number that goes up whenever the project changes, including by undo and redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether every commit checks the project's invariants. On in debug builds and deterministic sessions.
    pub fn validates(&self) -> bool {
        self.validate
    }

    /// Turns the invariant check after every commit on or off.
    pub fn set_validate(&mut self, validate: bool) {
        self.validate = validate;
    }

    /// Sets how much undo history to keep.
    pub fn set_history_limits(&mut self, limits: HistoryLimits) {
        self.history.set_limits(limits);
    }

    /// Labels of the steps that can be undone, oldest first.
    pub fn undo_labels(&self) -> impl Iterator<Item = &str> {
        self.history.undo_labels()
    }

    /// Labels of the steps that can be redone, next first.
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.history.redo_labels()
    }

    /// The events queued since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Records where the audio engine's playhead is and whether it plays. Not a command: it changes nothing in the
    /// project and queues no events.
    pub fn set_playhead(&mut self, position: Tick, playing: bool) {
        self.transport.position = position;
        self.transport.playing = playing;
    }

    /// Runs command `id` with `params`.
    ///
    /// Complexity: the command's own cost, plus O(N log N) in the project's entities when invariants are checked.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<CommandOutput, CommandError> {
        self.invoke(&Call::new(id, params))
    }

    /// Runs `call`.
    ///
    /// Complexity: the command's own cost, plus O(N log N) in the project's entities when invariants are checked.
    pub fn invoke(&mut self, call: &Call) -> Result<CommandOutput, CommandError> {
        let registry = Arc::clone(&self.registry);
        let spec = registry.get(&call.id).ok_or_else(|| unknown_command(&call.id))?;
        match &spec.handler {
            Handler::Session(run) => {
                self.history.seal();
                let revision = self.revision;
                let result = run(self, call.params.clone())?;
                Ok(CommandOutput { id: call.id.clone(), result, undo: None, changed: self.revision != revision, revision: self.revision })
            }
            Handler::Edit(run) => {
                let mut ctx = self.ctx();
                let result = run(&mut ctx, call.params.clone())?;
                let (outcome, label) = ctx.finish();
                let merge = spec.merge_key.zip(call.gesture).map(|(key, gesture)| format!("{}#{}#{gesture}", spec.id, key(&call.params)));
                let undo = self.commit(outcome, label.unwrap_or_else(|| spec.title.to_owned()), merge, spec.effects)?;
                Ok(CommandOutput { id: call.id.clone(), result, changed: undo.is_some(), undo, revision: self.revision })
            }
        }
    }

    /// Runs `calls` as one transaction and one undo step labeled `label` (or the commands' titles). Each command sees
    /// the changes of the ones before it. If any fails, nothing changes.
    ///
    /// Complexity: the commands' own costs, plus one invariant check.
    pub fn batch(&mut self, calls: &[Call], label: Option<&str>) -> Result<BatchOutput, BatchError> {
        let registry = Arc::clone(&self.registry);
        let mut ctx = self.ctx();
        let mut results = Vec::with_capacity(calls.len());
        let mut effects = Effects::NONE;
        let mut titles = Vec::with_capacity(calls.len());
        for (index, call) in calls.iter().enumerate() {
            let fail = |error| BatchError { index, id: call.id.clone(), error };
            let spec = registry.get(&call.id).ok_or_else(|| fail(unknown_command(&call.id)))?;
            let Handler::Edit(run) = &spec.handler else {
                return Err(fail(
                    CommandError::new(ErrorCode::NotInBatch, format!("{} acts on the session, so it can't run inside a batch", call.id))
                        .with_hint("run it on its own before or after the batch"),
                ));
            };
            results.push(run(&mut ctx, call.params.clone()).map_err(fail)?);
            effects.undoable |= spec.effects.undoable;
            titles.push(ctx.label.take().unwrap_or_else(|| spec.title.to_owned()));
        }
        let label = label.map(str::to_owned).unwrap_or_else(|| match titles.as_slice() {
            [one] => one.clone(),
            many => format!("{} ({} steps)", many.first().cloned().unwrap_or_default(), many.len()),
        });
        let (outcome, _) = ctx.finish();
        let undo = self.commit(outcome, label, None, effects).map_err(|error| BatchError { index: calls.len(), id: String::from("batch"), error })?;
        Ok(BatchOutput { results, changed: undo.is_some(), undo, revision: self.revision })
    }

    /// Undoes the newest step and returns its label, or `None` when there is nothing to undo.
    pub fn undo(&mut self) -> Option<String> {
        self.history.seal();
        let (project, label) = self.history.undo(Arc::clone(&self.project))?;
        self.restore(project);
        Some(label)
    }

    /// Redoes the newest undone step and returns its label, or `None` when there is nothing to redo.
    pub fn redo(&mut self) -> Option<String> {
        let (project, label) = self.history.redo(Arc::clone(&self.project))?;
        self.restore(project);
        Some(label)
    }

    fn restore(&mut self, project: Arc<Project>) {
        self.project = project;
        self.revision += 1;
        self.editor.sanitize(&self.project);
    }

    fn ctx(&mut self) -> Ctx<'_> {
        Ctx {
            project: (*self.project).clone(),
            editor: self.editor.clone(),
            transport: self.transport,
            env: &self.env,
            file: self.file.as_deref(),
            ids: &mut self.ids,
            events: Vec::new(),
            label: None,
        }
    }

    /// Makes an edit's outcome current. Returns the undo label when the project changed.
    fn commit(&mut self, outcome: Outcome, label: String, merge: Option<String>, effects: Effects) -> Result<Option<String>, CommandError> {
        let changed = !same_snapshot(&self.project, &outcome.project);
        if changed && !effects.undoable {
            return Err(CommandError::new(ErrorCode::Internal, "a command that isn't undoable tried to change the project; nothing changed"));
        }
        if changed && self.validate {
            outcome.project.validate().map_err(|e| {
                CommandError::rejected(format!("the edit would break the project: {e}"))
                    .with_candidates(e.0.iter().map(ToString::to_string).collect())
            })?;
        }
        let mut undo = None;
        if changed {
            let before = std::mem::replace(&mut self.project, Arc::new(outcome.project));
            self.history.commit(label.clone(), before, &self.project, merge);
            self.revision += 1;
            undo = Some(label);
        }
        self.editor = outcome.editor;
        self.editor.sanitize(&self.project);
        self.transport = outcome.transport;
        self.events.extend(outcome.events);
        Ok(undo)
    }

    /// Makes `project` the current one, opened from or saved as `file`, with no history.
    pub(crate) fn replace(&mut self, project: Project, file: Option<PathBuf>, saved: bool) {
        let project = Arc::new(project);
        self.saved = saved.then(|| Arc::clone(&project));
        self.project = project;
        self.file = file;
        self.history.clear();
        self.editor = EditorState::default();
        self.editor.sanitize(&self.project);
        self.editor.track = self.project.track_order.front().copied();
        self.editor.insert = self.project.mixer.order.front().copied();
        self.transport = TransportState { mode: self.transport.mode, ..TransportState::default() };
        self.revision += 1;
        self.events.push(Event::Stop);
        self.events.push(Event::ProjectReplaced);
    }

    /// Makes `project` current as an unsaved project with no file, for example one recovered from an autosave.
    pub fn recover(&mut self, project: Project) {
        self.replace(project, None, false);
    }

    /// Starts a new project that copies `project`, such as a template, with a new id and no file, replacing the
    /// current one.
    pub(crate) fn start_from(&mut self, mut project: Project) {
        project.id = self.ids.project();
        let created = (!self.ids.is_counting()).then(crate::template::now_rfc3339);
        project.meta = Arc::new(bodo_model::project::Meta { created, ..(*project.meta).clone() });
        self.replace(project, None, true);
    }

    /// Starts a new project from `template`, replacing the current one.
    pub fn new_project(&mut self, template: Template) {
        let created = (!self.ids.is_counting()).then(crate::template::now_rfc3339);
        let (project, editor) = new_project(template, &mut self.ids, &self.env, created);
        self.replace(project, None, true);
        self.editor = editor;
    }

    /// Records that the current project now lives in `file`, as saved there: it is no longer dirty. `project` is the
    /// saved version, whose sample paths may have been rebased to the new folder; the history follows it.
    pub(crate) fn mark_saved(&mut self, project: Project, file: PathBuf) {
        if project != *self.project {
            let before = Arc::clone(&self.project);
            let rebased: std::collections::HashMap<String, String> = before
                .samples
                .values()
                .filter_map(|old| project.samples.get(&old.id).filter(|new| new.path != old.path).map(|new| (old.path.clone(), new.path.clone())))
                .collect();
            if !rebased.is_empty() {
                self.history.rewrite(|snapshot| {
                    let mut changed = snapshot.clone();
                    let mut any = false;
                    for sample in snapshot.samples.values() {
                        if let Some(path) = rebased.get(&sample.path) {
                            let mut moved = (**sample).clone();
                            moved.path.clone_from(path);
                            changed.samples.insert(moved.id, Arc::new(moved));
                            any = true;
                        }
                    }
                    any.then_some(changed)
                });
            }
            self.project = Arc::new(project);
            self.revision += 1;
        }
        self.saved = Some(Arc::clone(&self.project));
        self.file = Some(file.clone());
        self.events.push(Event::Saved(file));
    }
}

fn unknown_command(id: &str) -> CommandError {
    CommandError::new(ErrorCode::UnknownCommand, format!("no command `{id}`")).with_hint("list_commands shows every command")
}
