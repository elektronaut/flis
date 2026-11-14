use std::fmt;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Resolved,
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Cost {
    /// A few minutes lost
    #[default]
    Minor,
    /// A real detour: a long debugging session or a redo
    Major,
    /// Stopped the task until a way around was found
    Blocker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum FixLocation {
    /// The shared setup: dotfiles, Claude instructions, settings, hooks, skills
    Setup,
    /// One of the user's own repositories
    Project,
    /// Someone else's software, Claude Code included
    Upstream,
    /// Nothing to change
    #[value(skip)]
    None,
}

macro_rules! text_enum {
    ($ty:ty { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $ty {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(raw: &str) -> Option<Self> {
                match raw { $($text => Some(Self::$variant),)+ _ => None }
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

text_enum!(Status { Open => "open", Resolved => "resolved", Accepted => "accepted" });
text_enum!(Cost { Minor => "minor", Major => "major", Blocker => "blocker" });
text_enum!(FixLocation { Setup => "setup", Project => "project", Upstream => "upstream", None => "none" });

impl Cost {
    pub fn weight(self) -> u64 {
        match self {
            Self::Minor => 1,
            Self::Major => 3,
            Self::Blocker => 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub symptom: String,
    pub workaround: Option<String>,
    pub hypothesis: Option<String>,
    pub fix: Option<String>,
    pub subject: Option<String>,
    pub silent: bool,
    pub status: Status,
    pub fix_location: Option<FixLocation>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub resolution_note: Option<String>,
    pub resolved_at: Option<String>,
    pub regressions: i64,
    pub merged_into: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl Entry {
    pub fn is_waiting(&self) -> bool {
        self.status == Status::Open && self.reference.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Occurrence {
    pub id: i64,
    pub entry_id: i64,
    pub at: String,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub agent: Option<String>,
    pub session: Option<String>,
    pub cost: Cost,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryRecord {
    pub field: String,
    pub old_value: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Default)]
pub struct Origin {
    pub project: Option<String>,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub agent: Option<String>,
    pub session: Option<String>,
}
