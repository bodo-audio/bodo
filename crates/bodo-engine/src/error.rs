//! What a command reports when it can't run, written for people and agents alike.
//!
//! Every error has a [`ErrorCode`] an agent can branch on, a message that names the entity involved, and often a hint
//! with what to do instead. An ambiguous name lists its candidates (project.md section 8.2).

use core::fmt;

/// What kind of problem a command hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No command has that id.
    UnknownCommand,
    /// The params don't match the command's schema, or a value is out of range.
    InvalidParams,
    /// A reference names nothing.
    NotFound,
    /// A name matches more than one entity.
    Ambiguous,
    /// The command acts on the selection, and nothing suitable is selected.
    NothingSelected,
    /// The edit isn't allowed: it would break an invariant, the target is locked, or work would be lost.
    Rejected,
    /// A file could not be read or written.
    Io,
    /// The command changes the session itself, so it can't run inside a batch.
    NotInBatch,
    /// Something went wrong inside Bodo. The project is unchanged.
    Internal,
}

impl ErrorCode {
    /// The code as it appears in messages, such as `not_found`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCommand => "unknown_command",
            Self::InvalidParams => "invalid_params",
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
            Self::NothingSelected => "nothing_selected",
            Self::Rejected => "rejected",
            Self::Io => "io",
            Self::NotInBatch => "not_in_batch",
            Self::Internal => "internal",
        }
    }
}

/// Why a command didn't run. The project is unchanged whenever a command returns one.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CommandError {
    /// What kind of problem it is.
    pub code: ErrorCode,
    /// What went wrong, naming the entities involved.
    pub message: String,
    /// What to do instead, when there is something useful to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// For an ambiguous or unknown name: the entities it could mean.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

impl CommandError {
    /// An error with `code` and `message`.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), hint: None, candidates: Vec::new() }
    }

    /// Params that don't fit the command.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, message)
    }

    /// An edit that isn't allowed.
    pub fn rejected(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Rejected, message)
    }

    /// A reference to a `kind` of entity, written `text`, that names nothing.
    pub fn not_found(kind: &str, text: &str) -> Self {
        Self::new(ErrorCode::NotFound, format!("no {kind} matches `{text}`"))
    }

    /// A command that needs a selected `what` when none is selected.
    pub fn nothing_selected(what: &str) -> Self {
        Self::new(ErrorCode::NothingSelected, format!("no {what} given and none selected")).with_hint(format!("pass the {what} explicitly"))
    }

    /// A failed file operation on `path`.
    pub fn io(path: &std::path::Path, error: impl fmt::Display) -> Self {
        Self::new(ErrorCode::Io, format!("{}: {error}", path.display()))
    }

    /// The same error with a hint.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The same error with candidates.
    pub fn with_candidates(mut self, candidates: Vec<String>) -> Self {
        self.candidates = candidates;
        self
    }
}

impl fmt::Display for CommandError {
    /// Compiler style: `error[not_found]: no channel matches `Kik``, then the candidates and the hint on their own lines.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code.as_str(), self.message)?;
        if !self.candidates.is_empty() {
            write!(f, "\n  candidates: {}", self.candidates.join(", "))?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_print_like_a_compiler() {
        let error = CommandError::new(ErrorCode::Ambiguous, "`Kick` names 2 channels")
            .with_candidates(vec!["ch_1 (Kick)".into(), "ch_2 (Kick)".into()])
            .with_hint("use an id or channel:N");
        assert_eq!(
            error.to_string(),
            "error[ambiguous]: `Kick` names 2 channels\n  candidates: ch_1 (Kick), ch_2 (Kick)\n  hint: use an id or channel:N"
        );
    }
}
