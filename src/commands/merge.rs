use std::fmt::Write;

use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::Context;
use crate::error::{Error, Result};
use crate::model::{Entry, Status};
use crate::output::{Report, plural, seen_text};
use crate::store::{self, Seen};

#[derive(Debug, Serialize)]
pub struct MergedSource {
    pub id: i64,
    /// The id as given, when it was itself merged away and led to `id`.
    pub given: Option<i64>,
    pub occurrences: usize,
}

#[derive(Debug, Serialize)]
pub struct MergeReport {
    pub entry: Entry,
    pub merged_from: Vec<i64>,
    pub merged: Vec<MergedSource>,
    /// Ids given that had already been merged into the target.
    pub already_merged: Vec<i64>,
    pub seen: Seen,
}

pub fn run(conn: &mut Connection, ctx: &Context, ids: &[i64], into: i64) -> Result<MergeReport> {
    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (target, merged_from) = store::resolve_entry(&tx, into)?;

    let mut sources: Vec<(Entry, Option<i64>)> = Vec::new();
    let mut already_merged = Vec::new();
    for &id in ids {
        let (source, followed) = store::resolve_entry(&tx, id)?;
        if source.id == target.id {
            if followed.is_empty() {
                let via = if into == target.id {
                    String::new()
                } else {
                    format!(" ({into} was merged into {})", target.id)
                };
                return Err(Error::usage(format!("cannot merge {id} into itself{via}")));
            }
            if !already_merged.contains(&id) {
                already_merged.push(id);
            }
            continue;
        }
        if sources.iter().any(|(known, _)| known.id == source.id) {
            continue;
        }
        let given = (!followed.is_empty()).then_some(id);
        sources.push((source, given));
    }

    let mut merged = Vec::new();
    for (source, given) in sources {
        if let Some(first) = store::occurrences(&tx, source.id)?.first() {
            let block = source_text(&source);
            let note = match first.note.as_deref() {
                Some(note) => format!("{note}\n\n{block}"),
                None => block,
            };
            store::set_occurrence_note(&tx, first.id, &note)?;
        }
        let occurrences = store::move_occurrences(&tx, source.id, target.id, None)?;
        store::set_merged_into(&tx, source.id, target.id, &now)?;
        store::reindex(&tx, source.id)?;
        merged.push(MergedSource {
            id: source.id,
            given,
            occurrences,
        });
    }
    if !merged.is_empty() {
        store::touch(&tx, target.id, &now)?;
    }
    store::reindex(&tx, target.id)?;
    let seen = store::seen_summary(&tx, target.id)?;
    let (entry, _) = store::resolve_entry(&tx, target.id)?;
    tx.commit()?;
    Ok(MergeReport {
        entry,
        merged_from,
        merged,
        already_merged,
        seen,
    })
}

/// The fields of a merged entry, kept on its first occurrence.
fn source_text(entry: &Entry) -> String {
    let mut out = format!("Merged from {}: {}", entry.id, entry.symptom);
    let fields = [
        ("workaround", &entry.workaround),
        ("hypothesis", &entry.hypothesis),
        ("fix", &entry.fix),
        ("subject", &entry.subject),
        ("ref", &entry.reference),
    ];
    for (label, value) in fields {
        if let Some(value) = value {
            let _ = write!(out, "\n{label}: {value}");
        }
    }
    if entry.silent {
        out.push_str("\nsilent: true");
    }
    if entry.status != Status::Open {
        let _ = write!(out, "\nstatus: {}", entry.status);
        if let Some(note) = &entry.resolution_note {
            let _ = write!(out, " ({note})");
        }
    }
    out
}

impl Report for MergeReport {
    fn text(&self) -> String {
        let id = self.entry.id;
        let mut out = String::new();
        for old in &self.merged_from {
            let _ = writeln!(out, "{old} was merged into {id}.");
        }
        for old in &self.already_merged {
            let _ = writeln!(out, "{old} was already merged into {id}.");
        }
        if self.merged.is_empty() {
            let _ = writeln!(out, "Nothing to merge into {id}.");
            return out;
        }
        let ids: Vec<String> = self
            .merged
            .iter()
            .map(|source| match source.given {
                Some(given) => format!("{} (via {given})", source.id),
                None => source.id.to_string(),
            })
            .collect();
        let moved: usize = self.merged.iter().map(|source| source.occurrences).sum();
        let _ = writeln!(
            out,
            "Merged {} into {id}: moved {}. {id} is {}, {}.",
            ids.join(", "),
            plural(moved as u64, "occurrence", "occurrences"),
            self.entry.status,
            seen_text(&self.seen)
        );
        let _ = writeln!(
            out,
            "The old ids lead to {id}. To undo part of it: flis split {id} --occurrence <oid>... --symptom \"…\""
        );
        out
    }
}
