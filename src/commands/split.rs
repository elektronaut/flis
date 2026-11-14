use std::fmt::Write;

use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::{Context, clean};
use crate::error::{Error, Result};
use crate::model::Entry;
use crate::output::{Report, plural};
use crate::store::{self, NewEntry, Seen};

#[derive(Debug, Serialize)]
pub struct SplitReport {
    pub id: i64,
    pub entry: Entry,
    pub source: Entry,
    pub merged_from: Vec<i64>,
    pub moved: Vec<i64>,
    pub seen: Seen,
    pub source_seen: Seen,
}

pub fn run(
    conn: &mut Connection,
    ctx: &Context,
    id: i64,
    occurrence_ids: &[i64],
    symptom: &str,
) -> Result<SplitReport> {
    let symptom = clean(Some(symptom))
        .ok_or_else(|| Error::usage("--symptom describes the problem the occurrences belong to"))?;
    let mut moved: Vec<i64> = Vec::new();
    for occurrence in occurrence_ids {
        if !moved.contains(occurrence) {
            moved.push(*occurrence);
        }
    }
    if moved.is_empty() {
        return Err(Error::usage("name at least one --occurrence to move"));
    }

    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (source, merged_from) = store::resolve_entry(&tx, id)?;
    for occurrence_id in &moved {
        let occurrence = store::find_occurrence(&tx, *occurrence_id)?
            .ok_or_else(|| Error::NotFound(format!("no occurrence o{occurrence_id}")))?;
        if occurrence.entry_id != source.id {
            return Err(Error::usage(format!(
                "o{occurrence_id} belongs to entry {}, not {}",
                occurrence.entry_id, source.id
            )));
        }
    }
    if store::seen_summary(&tx, source.id)?.count <= moved.len() as i64 {
        return Err(Error::usage(format!(
            "that is every occurrence of {}; an entry keeps at least one. \
             To describe it differently, run `flis edit {} --symptom \"…\"`",
            source.id, source.id
        )));
    }

    let new_entry = NewEntry {
        symptom,
        workaround: None,
        hypothesis: None,
        fix: None,
        subject: source.subject.clone(),
        silent: false,
    };
    let new_id = store::insert_entry(&tx, &new_entry, &now)?;
    store::move_occurrences(&tx, source.id, new_id, Some(&moved))?;
    store::touch(&tx, source.id, &now)?;
    store::reindex(&tx, source.id)?;
    store::reindex(&tx, new_id)?;
    let seen = store::seen_summary(&tx, new_id)?;
    let source_seen = store::seen_summary(&tx, source.id)?;
    let (entry, _) = store::resolve_entry(&tx, new_id)?;
    let (source, _) = store::resolve_entry(&tx, source.id)?;
    tx.commit()?;
    Ok(SplitReport {
        id: new_id,
        entry,
        source,
        merged_from,
        moved,
        seen,
        source_seen,
    })
}

impl Report for SplitReport {
    fn text(&self) -> String {
        let mut out = String::new();
        for old in &self.merged_from {
            let _ = writeln!(out, "{old} was merged into {}.", self.source.id);
        }
        let moved: Vec<String> = self.moved.iter().map(|id| format!("o{id}")).collect();
        let _ = writeln!(
            out,
            "Split {} from {} into new entry {}. {} keeps {}.",
            moved.join(", "),
            self.source.id,
            self.id,
            self.source.id,
            plural(
                self.source_seen.count.max(0) as u64,
                "occurrence",
                "occurrences"
            ),
        );
        let _ = writeln!(
            out,
            "Add what is known about it: flis edit {} --workaround \"…\" --hypothesis \"…\"",
            self.id
        );
        out
    }
}
