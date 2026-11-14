use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::seen::reopen_changes;
use super::{Context, clean};
use crate::error::{Error, Result};
use crate::model::{Entry, FixLocation, Status};
use crate::output::Report;
use crate::store::{self, Field};

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub entry: Entry,
    pub merged_from: Vec<i64>,
    pub previous_status: Status,
}

pub fn resolve(
    conn: &mut Connection,
    ctx: &Context,
    id: i64,
    location: FixLocation,
    reference: &str,
    note: Option<&str>,
) -> Result<StatusReport> {
    let reference = clean(Some(reference)).ok_or_else(|| {
        Error::usage("--ref names the commit, PR, issue, file or rule with the fix")
    })?;
    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (entry, merged_from) = store::resolve_entry(&tx, id)?;
    if entry.status == Status::Resolved {
        return Err(Error::usage(format!(
            "entry {} is already resolved (ref {}); run `flis reopen {}` first to resolve it again",
            entry.id,
            entry.reference.as_deref().unwrap_or("none"),
            entry.id
        )));
    }
    let changes = [
        (Field::Status, Some(Status::Resolved.to_string())),
        (Field::FixLocation, Some(location.to_string())),
        (Field::Ref, Some(reference)),
        (Field::ResolutionNote, clean(note)),
        (Field::ResolvedAt, Some(now.clone())),
    ];
    store::update_fields(&tx, &entry, &changes, &now)?;
    finish(tx, entry, merged_from)
}

pub fn accept(conn: &mut Connection, ctx: &Context, id: i64, note: &str) -> Result<StatusReport> {
    let note = clean(Some(note))
        .ok_or_else(|| Error::usage("--note gives the reason the problem is accepted"))?;
    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (entry, merged_from) = store::resolve_entry(&tx, id)?;
    if entry.status != Status::Open {
        return Err(Error::usage(format!(
            "entry {} is {}; run `flis reopen {}` first to accept it",
            entry.id, entry.status, entry.id
        )));
    }
    let changes = [
        (Field::Status, Some(Status::Accepted.to_string())),
        (Field::FixLocation, Some(FixLocation::None.to_string())),
        (Field::ResolutionNote, Some(note)),
    ];
    store::update_fields(&tx, &entry, &changes, &now)?;
    finish(tx, entry, merged_from)
}

pub fn reopen(conn: &mut Connection, ctx: &Context, id: i64) -> Result<StatusReport> {
    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (entry, merged_from) = store::resolve_entry(&tx, id)?;
    if entry.status != Status::Open {
        store::update_fields(&tx, &entry, &reopen_changes(None), &now)?;
    }
    finish(tx, entry, merged_from)
}

fn finish(
    tx: rusqlite::Transaction<'_>,
    before: Entry,
    merged_from: Vec<i64>,
) -> Result<StatusReport> {
    store::reindex(&tx, before.id)?;
    let (entry, _) = store::resolve_entry(&tx, before.id)?;
    tx.commit()?;
    Ok(StatusReport {
        entry,
        merged_from,
        previous_status: before.status,
    })
}

impl Report for StatusReport {
    fn text(&self) -> String {
        let mut out = String::new();
        for id in &self.merged_from {
            out.push_str(&format!("{id} was merged into {}.\n", self.entry.id));
        }
        let id = self.entry.id;
        let line = match (self.previous_status, self.entry.status) {
            (Status::Open, Status::Open) => format!("{id} is already open."),
            (_, Status::Resolved) => format!(
                "Resolved {id} (in {}, ref {}).",
                self.entry.fix_location.map_or("?", FixLocation::as_str),
                self.entry.reference.as_deref().unwrap_or("?")
            ),
            (_, Status::Accepted) => format!(
                "Accepted {id}: {}",
                self.entry.resolution_note.as_deref().unwrap_or("?")
            ),
            (previous, _) => format!(
                "Reopened {id} (was {previous}). The earlier resolution is in `flis show {id} --history`."
            ),
        };
        out.push_str(&line);
        out.push('\n');
        out
    }
}
