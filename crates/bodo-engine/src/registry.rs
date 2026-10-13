//! The command registry (project.md section 12.1, ADR 0005).
//!
//! Every change to a project is a registered command: an id such as `channel.add`, a JSON Schema for its params, its
//! effects, and a handler. Menus, shortcuts, the command palette, MCP and the assistant all read this one registry.
//!
//! Most handlers are edits: they get a scratch copy of the project and the editor state in a [`Ctx`] and change it;
//! the session commits the result as one transaction. A few handlers act on the session itself (undo, open, save) and
//! can't run inside a batch.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::CommandError;
use crate::session::{Ctx, Session};

/// What running a command can do. The engine's permission policy keys on these (project.md section 13.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effects {
    /// It changes the project and makes an undo step.
    pub undoable: bool,
    /// It can lose work that undo can't bring back, such as replacing the project.
    pub destructive: bool,
    /// It writes files.
    pub disk: bool,
    /// It uses the network.
    pub network: bool,
}

impl Effects {
    /// Changes nothing in the project: the selection, the transport, a query.
    pub const NONE: Self = Self { undoable: false, destructive: false, disk: false, network: false };
    /// An ordinary edit.
    pub const UNDOABLE: Self = Self { undoable: true, destructive: false, disk: false, network: false };
    /// Replaces the project or loses unsaved work.
    pub const DESTRUCTIVE: Self = Self { undoable: false, destructive: true, disk: false, network: false };
    /// Writes files.
    pub const DISK: Self = Self { undoable: false, destructive: false, disk: true, network: false };

    /// The effects as words, such as `undoable, disk`, or `none`.
    pub fn describe(self) -> String {
        let words: Vec<&str> = [(self.undoable, "undoable"), (self.destructive, "destructive"), (self.disk, "disk"), (self.network, "network")]
            .into_iter()
            .filter_map(|(on, word)| on.then_some(word))
            .collect();
        if words.is_empty() { String::from("none") } else { words.join(", ") }
    }
}

/// Derives what a command acts on from its params, such as a channel id. Two calls in one gesture with the same key
/// fold into one undo step.
pub type MergeKeyFn = fn(&Value) -> String;

type EditFn = dyn Fn(&mut Ctx<'_>, Value) -> Result<Value, CommandError> + Send + Sync;
type SessionFn = dyn Fn(&mut Session, Value) -> Result<Value, CommandError> + Send + Sync;

/// How a command runs.
pub(crate) enum Handler {
    /// A transactional edit of the project, the editor state or the transport.
    Edit(Box<EditFn>),
    /// An action on the session itself: history and files.
    Session(Box<SessionFn>),
}

/// A registered command.
pub struct CommandSpec {
    /// The id, `namespace.verb`.
    pub id: &'static str,
    /// A short title for menus, the palette and undo labels, such as "Add channel".
    pub title: &'static str,
    /// One line for agents and the palette.
    pub summary: &'static str,
    /// The JSON Schema of the params.
    pub params: Value,
    /// What running it can do.
    pub effects: Effects,
    /// Folds a gesture into one undo step, for commands driven by drags.
    pub merge_key: Option<MergeKeyFn>,
    /// Where it appears in the main menu, such as `File/Save`.
    pub menu: Option<&'static str>,
    /// Its default shortcut, such as `Mod+S` (Mod is Cmd on macOS and Ctrl elsewhere) or `Space`.
    pub shortcut: Option<&'static str>,
    /// Example params, for documentation and tests.
    pub example: Option<Value>,
    pub(crate) handler: Handler,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSpec").field("id", &self.id).field("effects", &self.effects).finish_non_exhaustive()
    }
}

impl CommandSpec {
    /// Whether it acts on the session itself, so it can't run inside a batch.
    pub fn is_session_command(&self) -> bool {
        matches!(self.handler, Handler::Session(_))
    }

    /// The namespace, such as `channel`.
    pub fn namespace(&self) -> &'static str {
        self.id.split_once('.').map_or(self.id, |(ns, _)| ns)
    }

    /// Sets example params.
    pub(crate) fn example(&mut self, example: Value) -> &mut Self {
        self.example = Some(example);
        self
    }

    /// Lets a gesture fold into one undo step, keyed by what `key` derives from the params.
    pub(crate) fn merge(&mut self, key: MergeKeyFn) -> &mut Self {
        self.merge_key = Some(key);
        self
    }

    /// Puts it in the main menu.
    pub(crate) fn menu(&mut self, path: &'static str) -> &mut Self {
        self.menu = Some(path);
        self
    }

    /// Gives it a default shortcut.
    pub(crate) fn shortcut(&mut self, shortcut: &'static str) -> &mut Self {
        self.shortcut = Some(shortcut);
        self
    }
}

/// Every command, by id.
#[derive(Default)]
pub struct Registry {
    commands: BTreeMap<&'static str, CommandSpec>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").field("commands", &self.commands.len()).finish()
    }
}

/// Reads params into `P`; missing or null params read as `{}`.
fn parse<P: DeserializeOwned>(id: &str, params: Value) -> Result<P, CommandError> {
    let params = if params.is_null() { Value::Object(serde_json::Map::new()) } else { params };
    serde_json::from_value(params)
        .map_err(|e| CommandError::invalid(format!("{id}: {e}")).with_hint(format!("see describe_command(\"{id}\") for the params")))
}

/// The JSON Schema of `P`, without the `$schema` and `title` keys, which tool catalogs don't need.
fn schema<P: schemars::JsonSchema>() -> Value {
    let mut schema = schemars::schema_for!(P).to_value();
    if let Some(map) = schema.as_object_mut() {
        map.remove("$schema");
        map.remove("title");
    }
    schema
}

impl Registry {
    /// Every built-in command.
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        crate::commands::register(&mut registry);
        registry
    }

    /// The command with `id`.
    ///
    /// Complexity: O(log n) in commands.
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.get(id)
    }

    /// Every command, sorted by id.
    pub fn iter(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.values()
    }

    /// The commands in `namespace`, sorted by id.
    ///
    /// Complexity: O(n) in commands.
    pub fn namespace<'a>(&'a self, namespace: &'a str) -> impl Iterator<Item = &'a CommandSpec> + 'a {
        self.commands.values().filter(move |c| c.namespace() == namespace)
    }

    /// The number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether there are no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Adds `spec`, replacing any command with the same id.
    fn add(&mut self, spec: CommandSpec) -> &mut CommandSpec {
        match self.commands.entry(spec.id) {
            Entry::Vacant(entry) => entry.insert(spec),
            Entry::Occupied(mut entry) => {
                entry.insert(spec);
                entry.into_mut()
            }
        }
    }

    /// Registers an edit whose params are `P`.
    pub(crate) fn edit<P, F>(&mut self, id: &'static str, title: &'static str, summary: &'static str, effects: Effects, f: F) -> &mut CommandSpec
    where
        P: DeserializeOwned + schemars::JsonSchema,
        F: Fn(&mut Ctx<'_>, P) -> Result<Value, CommandError> + Send + Sync + 'static,
    {
        let handler = Handler::Edit(Box::new(move |ctx: &mut Ctx<'_>, params: Value| f(ctx, parse(id, params)?)));
        self.add(CommandSpec {
            id,
            title,
            summary,
            params: schema::<P>(),
            effects,
            merge_key: None,
            menu: None,
            shortcut: None,
            example: None,
            handler,
        })
    }

    /// Registers a session command whose params are `P`.
    pub(crate) fn session<P, F>(&mut self, id: &'static str, title: &'static str, summary: &'static str, effects: Effects, f: F) -> &mut CommandSpec
    where
        P: DeserializeOwned + schemars::JsonSchema,
        F: Fn(&mut Session, P) -> Result<Value, CommandError> + Send + Sync + 'static,
    {
        let handler = Handler::Session(Box::new(move |session: &mut Session, params: Value| f(session, parse(id, params)?)));
        self.add(CommandSpec {
            id,
            title,
            summary,
            params: schema::<P>(),
            effects,
            merge_key: None,
            menu: None,
            shortcut: None,
            example: None,
            handler,
        })
    }
}

/// Params of commands that take none.
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}
