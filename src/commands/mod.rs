pub mod add;
pub mod search;
pub mod seen;
pub mod show;

use std::path::PathBuf;

use time::OffsetDateTime;

use crate::clock::format_timestamp;
use crate::context::locate;
use crate::error::{Error, Result};
use crate::model::Origin;

/// What a command needs from its surroundings, so tests can supply it.
pub struct Context {
    pub now: OffsetDateTime,
    pub cwd: PathBuf,
    pub agent: Option<String>,
    pub session: Option<String>,
}

impl Context {
    pub fn now_text(&self) -> String {
        format_timestamp(self.now)
    }

    /// Where an occurrence happened. An explicit project replaces the one
    /// derived from the repository; an empty one clears it.
    pub fn origin(&self, project: Option<&str>, agent: Option<&str>) -> Origin {
        let location = locate(&self.cwd);
        Origin {
            project: match project {
                Some(name) => clean(Some(name)),
                None => location.project,
            },
            branch: location.branch,
            cwd: location.cwd,
            agent: clean(agent)
                .or_else(|| clean(self.agent.as_deref()))
                .or_else(|| Some("claude-code".to_owned())),
            session: clean(self.session.as_deref()),
        }
    }
}

/// Trimmed text, or `None` when there is none.
pub fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A subject is one lowercase word, such as `bundler`, `gh` or `sandbox`.
pub fn normalize_subject(raw: Option<&str>) -> Result<Option<String>> {
    let Some(subject) = clean(raw) else {
        return Ok(None);
    };
    if subject.chars().any(char::is_whitespace) {
        return Err(Error::usage(format!(
            "--subject is one word naming the tool or area (bundler, gh, sandbox), not {subject:?}"
        )));
    }
    Ok(Some(subject.to_lowercase()))
}
