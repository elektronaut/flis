use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Usage(String),
    NotFound(String),
    Storage { path: PathBuf, detail: String },
    Sqlite(rusqlite::Error),
    Internal(String),
}

impl Error {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::NotFound(_) => 3,
            Self::Storage { .. } | Self::Sqlite(_) | Self::Internal(_) => 1,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Usage(_) => "usage",
            Self::NotFound(_) => "not_found",
            Self::Storage { .. } => "storage",
            Self::Sqlite(_) | Self::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::NotFound(message) | Self::Internal(message) => {
                f.write_str(message)
            }
            Self::Storage { path, detail } => write!(
                f,
                "cannot open the snags database for writing at {}: {detail}. \
                 In a sandboxed session, add {} to sandbox.filesystem.allowWrite, \
                 or point SNAGS_DB at a writable path",
                path.display(),
                path.parent().unwrap_or(path).display(),
            ),
            Self::Sqlite(error) => write!(f, "database error: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn is_write_denied(&self) -> bool {
        use rusqlite::ErrorCode::{CannotOpen, PermissionDenied, ReadOnly};
        matches!(
            self,
            Self::Sqlite(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(failure.code, ReadOnly | CannotOpen | PermissionDenied)
        )
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(format!("I/O error: {error}"))
    }
}
