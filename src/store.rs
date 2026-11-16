use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{Error, Result};
use crate::model::{Cost, Entry, FixLocation, HistoryRecord, Occurrence, Origin, Status};

const ENTRY_COLUMNS: &str = "id, symptom, workaround, hypothesis, fix, subject, silent, status, \
     fix_location, ref, resolution_note, resolved_at, regressions, merged_into, created_at, updated_at";

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<Entry> {
    let status: String = row.get(7)?;
    let fix_location: Option<String> = row.get(8)?;
    Ok(Entry {
        id: row.get(0)?,
        symptom: row.get(1)?,
        workaround: row.get(2)?,
        hypothesis: row.get(3)?,
        fix: row.get(4)?,
        subject: row.get(5)?,
        silent: row.get(6)?,
        status: Status::parse(&status).unwrap_or(Status::Open),
        fix_location: fix_location.as_deref().and_then(FixLocation::parse),
        reference: row.get(9)?,
        resolution_note: row.get(10)?,
        resolved_at: row.get(11)?,
        regressions: row.get(12)?,
        merged_into: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn occurrence_from_row(row: &Row<'_>) -> rusqlite::Result<Occurrence> {
    let cost: String = row.get(8)?;
    Ok(Occurrence {
        id: row.get(0)?,
        entry_id: row.get(1)?,
        at: row.get(2)?,
        project: row.get(3)?,
        branch: row.get(4)?,
        cwd: row.get(5)?,
        agent: row.get(6)?,
        session: row.get(7)?,
        cost: Cost::parse(&cost).unwrap_or_default(),
        note: row.get(9)?,
    })
}

pub fn find_entry(conn: &Connection, id: i64) -> Result<Option<Entry>> {
    let sql = format!("SELECT {ENTRY_COLUMNS} FROM entries WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], entry_from_row).optional()?)
}

pub fn entries_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<Entry>> {
    let sql = format!("SELECT {ENTRY_COLUMNS} FROM entries WHERE id = ?1");
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(entry) = stmt.query_row([id], entry_from_row).optional()? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

pub fn all_live_entries(conn: &Connection) -> Result<Vec<Entry>> {
    let sql = format!("SELECT {ENTRY_COLUMNS} FROM entries WHERE merged_into IS NULL ORDER BY id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], entry_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The entry an id refers to, following `merged_into` to the entry that
/// survived. Returns the entry and the ids that were followed to reach it.
pub fn resolve_entry(conn: &Connection, id: i64) -> Result<(Entry, Vec<i64>)> {
    let mut followed = Vec::new();
    let mut current = id;
    loop {
        let entry = find_entry(conn, current)?
            .ok_or_else(|| Error::NotFound(format!("no entry {current}")))?;
        match entry.merged_into {
            Some(target) if !followed.contains(&current) && followed.len() < 64 => {
                followed.push(current);
                current = target;
            }
            Some(_) => {
                return Err(Error::Internal(format!("entry {id} has a merge cycle")));
            }
            None => return Ok((entry, followed)),
        }
    }
}

pub struct NewEntry {
    pub symptom: String,
    pub workaround: Option<String>,
    pub hypothesis: Option<String>,
    pub fix: Option<String>,
    pub subject: Option<String>,
    pub silent: bool,
}

pub fn insert_entry(conn: &Connection, entry: &NewEntry, now: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO entries (symptom, workaround, hypothesis, fix, subject, silent, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            entry.symptom,
            entry.workaround,
            entry.hypothesis,
            entry.fix,
            entry.subject,
            entry.silent,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_occurrence(
    conn: &Connection,
    entry_id: i64,
    at: &str,
    origin: &Origin,
    cost: Cost,
    note: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO occurrences (entry_id, at, project, branch, cwd, agent, session, cost, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            entry_id,
            at,
            origin.project,
            origin.branch,
            origin.cwd,
            origin.agent,
            origin.session,
            cost.as_str(),
            note
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn occurrences(conn: &Connection, entry_id: i64) -> Result<Vec<Occurrence>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, entry_id, at, project, branch, cwd, agent, session, cost, note
         FROM occurrences WHERE entry_id = ?1 ORDER BY at, id",
    )?;
    let rows = stmt.query_map([entry_id], occurrence_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Occurrences at or after `since`, for every entry.
pub fn occurrences_since(conn: &Connection, since: &str) -> Result<Vec<Occurrence>> {
    let mut stmt = conn.prepare(
        "SELECT id, entry_id, at, project, branch, cwd, agent, session, cost, note
         FROM occurrences WHERE at >= ?1 ORDER BY at, id",
    )?;
    let rows = stmt.query_map([since], occurrence_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn find_occurrence(conn: &Connection, id: i64) -> Result<Option<Occurrence>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, entry_id, at, project, branch, cwd, agent, session, cost, note
         FROM occurrences WHERE id = ?1",
    )?;
    Ok(stmt.query_row([id], occurrence_from_row).optional()?)
}

pub fn set_occurrence_note(conn: &Connection, id: i64, note: &str) -> Result<()> {
    conn.execute(
        "UPDATE occurrences SET note = ?1 WHERE id = ?2",
        params![note, id],
    )?;
    Ok(())
}

/// Sets the cost of an occurrence and keeps its earlier cost in the entry's
/// history as the field `o<id> cost`. Returns that field name, or `None` when
/// the cost is already the same.
pub fn set_occurrence_cost(
    conn: &Connection,
    occurrence: &Occurrence,
    cost: Cost,
    now: &str,
) -> Result<Option<String>> {
    if occurrence.cost == cost {
        return Ok(None);
    }
    let field = format!("o{} cost", occurrence.id);
    conn.execute(
        "INSERT INTO entry_history (entry_id, field, old_value, at) VALUES (?1, ?2, ?3, ?4)",
        params![occurrence.entry_id, field, occurrence.cost.as_str(), now],
    )?;
    conn.execute(
        "UPDATE occurrences SET cost = ?1 WHERE id = ?2",
        params![cost.as_str(), occurrence.id],
    )?;
    Ok(Some(field))
}

/// Moves occurrences to another entry: all of `from`'s, or only the given ids.
pub fn move_occurrences(
    conn: &Connection,
    from: i64,
    to: i64,
    ids: Option<&[i64]>,
) -> Result<usize> {
    match ids {
        None => Ok(conn.execute(
            "UPDATE occurrences SET entry_id = ?1 WHERE entry_id = ?2",
            params![to, from],
        )?),
        Some(ids) => {
            let mut moved = 0;
            for id in ids {
                moved += conn.execute(
                    "UPDATE occurrences SET entry_id = ?1 WHERE entry_id = ?2 AND id = ?3",
                    params![to, from, id],
                )?;
            }
            Ok(moved)
        }
    }
}

pub fn set_merged_into(conn: &Connection, id: i64, target: i64, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE entries SET merged_into = ?1, updated_at = ?2 WHERE id = ?3",
        params![target, now, id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Seen {
    pub count: i64,
    pub last_seen: Option<String>,
}

pub fn seen_summary(conn: &Connection, entry_id: i64) -> Result<Seen> {
    let mut stmt =
        conn.prepare_cached("SELECT count(*), max(at) FROM occurrences WHERE entry_id = ?1")?;
    Ok(stmt.query_row([entry_id], |row| {
        Ok(Seen {
            count: row.get(0)?,
            last_seen: row.get(1)?,
        })
    })?)
}

pub fn history(conn: &Connection, entry_id: i64) -> Result<Vec<HistoryRecord>> {
    let mut stmt = conn.prepare(
        "SELECT field, old_value, at FROM entry_history WHERE entry_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([entry_id], |row| {
        Ok(HistoryRecord {
            field: row.get(0)?,
            old_value: row.get(1)?,
            at: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The fields of an entry that `update_fields` may change, by column name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Symptom,
    Workaround,
    Hypothesis,
    Fix,
    Subject,
    Silent,
    Status,
    FixLocation,
    Ref,
    ResolutionNote,
    ResolvedAt,
    Regressions,
}

impl Field {
    pub fn column(self) -> &'static str {
        match self {
            Self::Symptom => "symptom",
            Self::Workaround => "workaround",
            Self::Hypothesis => "hypothesis",
            Self::Fix => "fix",
            Self::Subject => "subject",
            Self::Silent => "silent",
            Self::Status => "status",
            Self::FixLocation => "fix_location",
            Self::Ref => "ref",
            Self::ResolutionNote => "resolution_note",
            Self::ResolvedAt => "resolved_at",
            Self::Regressions => "regressions",
        }
    }

    /// Fields that describe a resolution or a report. Their earlier value is
    /// kept only when there was one.
    fn is_resolution(self) -> bool {
        matches!(
            self,
            Self::FixLocation | Self::Ref | Self::ResolutionNote | Self::ResolvedAt
        )
    }

    fn current(self, entry: &Entry) -> Option<String> {
        match self {
            Self::Symptom => Some(entry.symptom.clone()),
            Self::Workaround => entry.workaround.clone(),
            Self::Hypothesis => entry.hypothesis.clone(),
            Self::Fix => entry.fix.clone(),
            Self::Subject => entry.subject.clone(),
            Self::Silent => Some(entry.silent.to_string()),
            Self::Status => Some(entry.status.to_string()),
            Self::FixLocation => entry.fix_location.map(|location| location.to_string()),
            Self::Ref => entry.reference.clone(),
            Self::ResolutionNote => entry.resolution_note.clone(),
            Self::ResolvedAt => entry.resolved_at.clone(),
            Self::Regressions => Some(entry.regressions.to_string()),
        }
    }
}

/// Sets each field to its new value, saving the previous value of every
/// field that actually changes to `entry_history` (resolution fields only
/// when they had a value). Returns the changed fields.
pub fn update_fields(
    conn: &Connection,
    entry: &Entry,
    changes: &[(Field, Option<String>)],
    now: &str,
) -> Result<Vec<Field>> {
    let mut changed = Vec::new();
    for (field, new_value) in changes {
        let old_value = field.current(entry);
        if &old_value == new_value {
            continue;
        }
        if old_value.is_some() || !field.is_resolution() {
            conn.execute(
                "INSERT INTO entry_history (entry_id, field, old_value, at) VALUES (?1, ?2, ?3, ?4)",
                params![entry.id, field.column(), old_value, now],
            )?;
        }
        let value: Option<rusqlite::types::Value> = new_value.as_ref().map(|value| match field {
            Field::Silent => rusqlite::types::Value::Integer(i64::from(value == "true")),
            Field::Regressions => rusqlite::types::Value::Integer(value.parse().unwrap_or(0)),
            _ => rusqlite::types::Value::Text(value.clone()),
        });
        conn.execute(
            &format!("UPDATE entries SET {} = ?1 WHERE id = ?2", field.column()),
            params![value, entry.id],
        )?;
        changed.push(*field);
    }
    if !changed.is_empty() {
        touch(conn, entry.id, now)?;
    }
    Ok(changed)
}

pub fn touch(conn: &Connection, entry_id: i64, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE entries SET updated_at = ?1 WHERE id = ?2",
        params![now, entry_id],
    )?;
    Ok(())
}

/// Rebuilds the search row of an entry from its fields and the notes of all
/// its occurrences.
pub fn reindex(conn: &Connection, entry_id: i64) -> Result<()> {
    conn.execute("DELETE FROM entries_fts WHERE rowid = ?1", [entry_id])?;
    conn.execute(
        "INSERT INTO entries_fts (rowid, symptom, workaround, hypothesis, fix, subject, notes)
         SELECT e.id, e.symptom, e.workaround, e.hypothesis, e.fix, e.subject,
                (SELECT group_concat(o.note, char(10)) FROM occurrences o WHERE o.entry_id = e.id)
         FROM entries e WHERE e.id = ?1",
        [entry_id],
    )?;
    Ok(())
}

pub fn subjects(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT subject, count(*) FROM entries
         WHERE subject IS NOT NULL AND merged_into IS NULL
         GROUP BY subject ORDER BY count(*) DESC, subject",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn occurrences_in_project(conn: &Connection, project: &str) -> Result<Vec<Occurrence>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, entry_id, at, project, branch, cwd, agent, session, cost, note
         FROM occurrences WHERE project = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([project], occurrence_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The ids of the entries merged directly into `id`.
pub fn merged_into(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare_cached("SELECT id FROM entries WHERE merged_into = ?1 ORDER BY id")?;
    let rows = stmt.query_map([id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn delete_occurrence(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM occurrences WHERE id = ?1", [id])?;
    Ok(())
}

/// Deletes an entry with its occurrences, history and search row. Entries
/// merged into it must be deleted first.
pub fn delete_entry(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM occurrences WHERE entry_id = ?1", [id])?;
    conn.execute("DELETE FROM entry_history WHERE entry_id = ?1", [id])?;
    conn.execute("DELETE FROM entries_fts WHERE rowid = ?1", [id])?;
    conn.execute("DELETE FROM entries WHERE id = ?1", [id])?;
    Ok(())
}
