use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::Context;
use crate::error::{Error, Result};
use crate::output::{Report, plural};
use crate::store;

#[derive(Debug)]
pub struct DeleteArgs<'a> {
    pub ids: &'a [i64],
    pub occurrences: &'a [i64],
    pub projects: &'a [String],
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct DeletedEntry {
    pub id: i64,
    pub occurrences: Vec<i64>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct MergedFrom {
    pub given: i64,
    pub id: i64,
}

#[derive(Debug, Serialize)]
pub struct DeletedStub {
    pub id: i64,
    pub merged_into: i64,
}

#[derive(Debug, Serialize)]
pub struct KeptEntry {
    pub id: i64,
    pub deleted: Vec<i64>,
    pub remaining: usize,
}

#[derive(Debug, Serialize)]
pub struct DeleteReport {
    pub dry_run: bool,
    /// Ids given that were merged away, each with the entry it led to.
    pub merged_from: Vec<MergedFrom>,
    pub entries: Vec<DeletedEntry>,
    pub stubs: Vec<DeletedStub>,
    pub kept: Vec<KeptEntry>,
    /// Every occurrence deleted, including those of deleted entries.
    pub occurrences: Vec<i64>,
}

pub fn run(conn: &mut Connection, ctx: &Context, args: &DeleteArgs<'_>) -> Result<DeleteReport> {
    if args.ids.is_empty() && args.occurrences.is_empty() && args.projects.is_empty() {
        return Err(Error::usage(
            "name what to delete: entry ids, --occurrence or --project",
        ));
    }

    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let mut doomed: BTreeSet<i64> = BTreeSet::new();
    let mut merged_from = Vec::new();
    for &id in args.ids {
        let (entry, _) = store::resolve_entry(&tx, id)?;
        let followed = MergedFrom {
            given: id,
            id: entry.id,
        };
        if id != entry.id && !merged_from.contains(&followed) {
            merged_from.push(followed);
        }
        doomed.insert(entry.id);
    }

    let mut selected: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for &id in args.occurrences {
        let occurrence = store::find_occurrence(&tx, id)?
            .ok_or_else(|| Error::NotFound(format!("no occurrence o{id}")))?;
        selected
            .entry(occurrence.entry_id)
            .or_default()
            .insert(occurrence.id);
    }
    for project in args.projects {
        for occurrence in store::occurrences_in_project(&tx, project)? {
            selected
                .entry(occurrence.entry_id)
                .or_default()
                .insert(occurrence.id);
        }
    }

    let mut kept = Vec::new();
    for (entry_id, ids) in &selected {
        if doomed.contains(entry_id) {
            continue;
        }
        let total = store::occurrences(&tx, *entry_id)?.len();
        if total <= ids.len() {
            doomed.insert(*entry_id);
        } else {
            kept.push(KeptEntry {
                id: *entry_id,
                deleted: ids.iter().copied().collect(),
                remaining: total - ids.len(),
            });
        }
    }

    let mut entries = Vec::new();
    for &id in &doomed {
        let occurrences = store::occurrences(&tx, id)?
            .into_iter()
            .map(|occurrence| occurrence.id)
            .collect();
        entries.push(DeletedEntry { id, occurrences });
    }

    let mut stubs = Vec::new();
    let mut frontier: Vec<i64> = doomed.iter().copied().collect();
    while let Some(target) = frontier.pop() {
        for id in store::merged_into(&tx, target)? {
            if doomed.insert(id) {
                stubs.push(DeletedStub {
                    id,
                    merged_into: target,
                });
                frontier.push(id);
            }
        }
    }
    stubs.sort_by_key(|stub| stub.id);
    kept.retain(|entry| !doomed.contains(&entry.id));

    let mut occurrences: Vec<i64> = entries
        .iter()
        .flat_map(|entry| entry.occurrences.iter().copied())
        .chain(kept.iter().flat_map(|entry| entry.deleted.iter().copied()))
        .collect();
    occurrences.sort_unstable();

    if !args.dry_run {
        for entry in &kept {
            for id in &entry.deleted {
                store::delete_occurrence(&tx, *id)?;
            }
            store::touch(&tx, entry.id, &now)?;
            store::reindex(&tx, entry.id)?;
        }
        let mut order: Vec<i64> = entries.iter().map(|entry| entry.id).collect();
        order.extend(stubs.iter().map(|stub| stub.id));
        while !order.is_empty() {
            let before = order.len();
            let mut waiting = Vec::new();
            for id in order {
                if store::merged_into(&tx, id)?.is_empty() {
                    store::delete_entry(&tx, id)?;
                } else {
                    waiting.push(id);
                }
            }
            if waiting.len() == before {
                return Err(Error::Internal(format!(
                    "entries {waiting:?} have a merge cycle"
                )));
            }
            order = waiting;
        }
        tx.commit()?;
    }

    Ok(DeleteReport {
        dry_run: args.dry_run,
        merged_from,
        entries,
        stubs,
        kept,
        occurrences,
    })
}

fn oids(ids: &[i64]) -> String {
    ids.iter()
        .map(|id| format!("o{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Report for DeleteReport {
    fn text(&self) -> String {
        let mut out = String::new();
        let (delete, keeps) = if self.dry_run {
            ("Would delete", "would keep")
        } else {
            ("Deleted", "keeps")
        };
        for followed in &self.merged_from {
            let _ = writeln!(out, "{} was merged into {}.", followed.given, followed.id);
        }
        if self.entries.is_empty() && self.kept.is_empty() {
            out.push_str("Nothing to delete.\n");
            return out;
        }
        for entry in &self.entries {
            if entry.occurrences.is_empty() {
                let _ = writeln!(out, "{delete} entry {}.", entry.id);
            } else {
                let _ = writeln!(
                    out,
                    "{delete} entry {} with {}: {}.",
                    entry.id,
                    plural(entry.occurrences.len() as u64, "occurrence", "occurrences"),
                    oids(&entry.occurrences)
                );
            }
        }
        for stub in &self.stubs {
            let _ = writeln!(
                out,
                "{delete} merge stub {} (merged into {}).",
                stub.id, stub.merged_into
            );
        }
        for entry in &self.kept {
            let _ = writeln!(
                out,
                "{delete} {} from entry {}, which {keeps} {}.",
                oids(&entry.deleted),
                entry.id,
                plural(entry.remaining as u64, "occurrence", "occurrences")
            );
        }
        if self.dry_run {
            out.push_str("Dry run: nothing was deleted. Without --dry-run there is no undo.\n");
        } else {
            out.push_str("There is no undo.\n");
        }
        out
    }
}
