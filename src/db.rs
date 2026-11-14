use std::fs::{DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::error::{Error, Result};

pub const SCHEMA_VERSION: i64 = 1;

/// `FLIS_DB`, else `$XDG_DATA_HOME/flis/flis.sqlite3`, else
/// `~/.local/share/flis/flis.sqlite3`.
pub fn resolve_path(get: impl Fn(&str) -> Option<String>) -> Result<PathBuf> {
    let set = |name: &str| get(name).filter(|value| !value.is_empty());
    if let Some(path) = set("FLIS_DB") {
        return Ok(PathBuf::from(path));
    }
    let data_home = match set("XDG_DATA_HOME").filter(|dir| Path::new(dir).is_absolute()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = set("HOME").ok_or_else(|| {
                Error::Internal("neither FLIS_DB, XDG_DATA_HOME nor HOME is set".into())
            })?;
            PathBuf::from(home).join(".local/share")
        }
    };
    Ok(data_home.join("flis").join("flis.sqlite3"))
}

pub fn open(path: &Path) -> Result<Connection> {
    let denied = |detail: String| Error::Storage {
        path: path.to_owned(),
        detail,
    };
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty())
        && !dir.is_dir()
    {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|error| denied(error.to_string()))?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|error| denied(error.to_string()))?;

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn =
        Connection::open_with_flags(path, flags).map_err(|error| denied(error.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match configure(&conn)
            .map_err(Error::from)
            .and_then(|()| migrate(&mut conn))
        {
            Ok(()) => return Ok(conn),
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(storage_error(path, error)),
        }
    }
}

/// Switching a new database to WAL takes a lock that the busy timeout does
/// not wait for, so a racing first open is retried.
fn is_busy(error: &Error) -> bool {
    matches!(
        error,
        Error::Sqlite(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(failure.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

pub fn storage_error(path: &Path, error: Error) -> Error {
    if error.is_write_denied() {
        Error::Storage {
            path: path.to_owned(),
            detail: error.to_string(),
        }
    } else {
        error
    }
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
    }
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")
}

fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn migrate(conn: &mut Connection) -> Result<()> {
    if user_version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version = user_version(&tx)?;
    if version > SCHEMA_VERSION {
        return Err(Error::Internal(format!(
            "the database has schema version {version}, newer than this flis supports ({SCHEMA_VERSION}); upgrade flis"
        )));
    }
    for (index, step) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(step)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
    }
    tx.commit()?;
    Ok(())
}

const MIGRATIONS: &[&str] = &[r"
CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    symptom TEXT NOT NULL,
    workaround TEXT,
    hypothesis TEXT,
    fix TEXT,
    subject TEXT,
    silent INTEGER NOT NULL DEFAULT 0 CHECK (silent IN (0, 1)),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved', 'accepted')),
    fix_location TEXT CHECK (fix_location IN ('setup', 'project', 'upstream', 'none')),
    ref TEXT,
    resolution_note TEXT,
    resolved_at TEXT,
    regressions INTEGER NOT NULL DEFAULT 0,
    merged_into INTEGER REFERENCES entries (id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX entries_subject ON entries (subject);

CREATE TABLE occurrences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id INTEGER NOT NULL REFERENCES entries (id),
    at TEXT NOT NULL,
    project TEXT,
    branch TEXT,
    cwd TEXT,
    agent TEXT,
    session TEXT,
    cost TEXT NOT NULL DEFAULT 'minor' CHECK (cost IN ('minor', 'major', 'blocker')),
    note TEXT
);

CREATE INDEX occurrences_entry_at ON occurrences (entry_id, at);
CREATE INDEX occurrences_at ON occurrences (at);

CREATE TABLE entry_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id INTEGER NOT NULL REFERENCES entries (id),
    field TEXT NOT NULL,
    old_value TEXT,
    at TEXT NOT NULL
);

CREATE INDEX entry_history_entry ON entry_history (entry_id);

CREATE VIRTUAL TABLE entries_fts USING fts5 (
    symptom, workaround, hypothesis, fix, subject, notes,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn path_prefers_flis_db_then_xdg_then_home() {
        let all = env(&[
            ("FLIS_DB", "/x/f.db"),
            ("XDG_DATA_HOME", "/xdg"),
            ("HOME", "/home/a"),
        ]);
        assert_eq!(resolve_path(all).unwrap(), PathBuf::from("/x/f.db"));
        let xdg = env(&[
            ("FLIS_DB", ""),
            ("XDG_DATA_HOME", "/xdg"),
            ("HOME", "/home/a"),
        ]);
        assert_eq!(
            resolve_path(xdg).unwrap(),
            PathBuf::from("/xdg/flis/flis.sqlite3")
        );
        let relative_xdg = env(&[("XDG_DATA_HOME", "rel"), ("HOME", "/home/a")]);
        assert_eq!(
            resolve_path(relative_xdg).unwrap(),
            PathBuf::from("/home/a/.local/share/flis/flis.sqlite3")
        );
        assert!(resolve_path(env(&[])).is_err());
    }

    #[test]
    fn creates_private_directory_and_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data/flis/flis.sqlite3");
        let conn = open(&path).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(mode(&path), 0o600);
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        drop(conn);
        assert_eq!(user_version(&open(&path).unwrap()).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn bundled_sqlite_has_fts5() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE t USING fts5(body);
             INSERT INTO t (body) VALUES ('bundler cannot find gem');",
        )
        .unwrap();
        let hits: i64 = conn
            .query_row("SELECT count(*) FROM t WHERE t MATCH '\"gem\"'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hits, 1);
    }

    #[test]
    fn unwritable_location_names_path_and_setting() {
        let temp = tempfile::tempdir().unwrap();
        let locked = temp.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let path = locked.join("flis.sqlite3");
        let message = open(&path).unwrap_err().to_string();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(
            message.contains("sandbox.filesystem.allowWrite"),
            "{message}"
        );
    }
}
