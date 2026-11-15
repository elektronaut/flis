use std::fmt::Write;

use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::{Context, clean};
use crate::error::Result;
use crate::model::{Cost, Entry, FixLocation, Status};
use crate::output::{Report, field, resolution_text, seen_text};
use crate::store::{self, Field, Seen};

pub struct SeenArgs<'a> {
    pub id: i64,
    pub note: Option<&'a str>,
    pub cost: Option<Cost>,
    pub project: Option<&'a str>,
    pub agent: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct Regression {
    pub count: i64,
    pub fix_location: Option<FixLocation>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub resolution_note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SeenReport {
    pub occurrence_id: i64,
    pub entry: Entry,
    pub merged_from: Vec<i64>,
    pub seen: Seen,
    pub regression: Option<Regression>,
}

pub fn run(conn: &mut Connection, ctx: &Context, args: &SeenArgs<'_>) -> Result<SeenReport> {
    let origin = ctx.origin(args.project, args.agent);
    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (entry, merged_from) = store::resolve_entry(&tx, args.id)?;
    let occurrence_id = store::insert_occurrence(
        &tx,
        entry.id,
        &now,
        &origin,
        args.cost.unwrap_or_default(),
        clean(args.note).as_deref(),
    )?;

    let mut regression = None;
    if entry.status == Status::Resolved {
        let count = entry.regressions + 1;
        store::update_fields(&tx, &entry, &reopen_changes(Some(count)), &now)?;
        regression = Some(Regression {
            count,
            fix_location: entry.fix_location,
            reference: entry.reference.clone(),
            resolution_note: entry.resolution_note.clone(),
        });
    } else {
        store::touch(&tx, entry.id, &now)?;
    }
    store::reindex(&tx, entry.id)?;
    let seen = store::seen_summary(&tx, entry.id)?;
    let (entry, _) = store::resolve_entry(&tx, entry.id)?;
    tx.commit()?;
    Ok(SeenReport {
        occurrence_id,
        entry,
        merged_from,
        seen,
        regression,
    })
}

/// The changes that make an entry open again, its resolution saved in the
/// history. `regressions` is set when the reopening is a regression.
pub fn reopen_changes(regressions: Option<i64>) -> Vec<(Field, Option<String>)> {
    let mut changes = vec![
        (Field::Status, Some(Status::Open.to_string())),
        (Field::FixLocation, None),
        (Field::Ref, None),
        (Field::ResolutionNote, None),
        (Field::ResolvedAt, None),
    ];
    if let Some(count) = regressions {
        changes.push((Field::Regressions, Some(count.to_string())));
    }
    changes
}

impl Report for SeenReport {
    fn text(&self) -> String {
        let entry = &self.entry;
        let mut out = String::new();
        for id in &self.merged_from {
            let _ = writeln!(out, "{id} was merged into {}.", entry.id);
        }
        let _ = writeln!(
            out,
            "Recorded o{} on {} ({}, {}).",
            self.occurrence_id,
            entry.id,
            entry.status,
            seen_text(&self.seen)
        );
        if let Some(regression) = &self.regression {
            let previous = Entry {
                fix_location: regression.fix_location,
                reference: regression.reference.clone(),
                resolution_note: regression.resolution_note.clone(),
                ..entry.clone()
            };
            let fixed = resolution_text(&previous).unwrap_or_else(|| "no details".to_owned());
            let _ = writeln!(
                out,
                "Regression {}: {} was resolved ({fixed}) and is open again.",
                regression.count, entry.id
            );
        }
        match &entry.workaround {
            Some(workaround) => field(&mut out, "", "workaround", workaround),
            None => {
                let _ = writeln!(
                    out,
                    "No workaround recorded. If you find one: snags edit {} --workaround \"…\"",
                    entry.id
                );
            }
        }
        out
    }
}
