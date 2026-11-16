use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::{Context, clean, normalize_subject};
use crate::error::{Error, Result};
use crate::model::{Cost, Entry, Occurrence, Status};
use crate::output::Report;
use crate::store::{self, Field};

/// The fields to change. `Some("")` clears an optional field. `cost` applies
/// to the occurrences in `occurrences`, or to all of the entry's when empty.
#[derive(Debug, Default)]
pub struct EditArgs {
    pub id: i64,
    pub symptom: Option<String>,
    pub workaround: Option<String>,
    pub hypothesis: Option<String>,
    pub fix: Option<String>,
    pub subject: Option<String>,
    pub silent: Option<bool>,
    pub reference: Option<String>,
    pub cost: Option<Cost>,
    pub occurrences: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct EditReport {
    pub entry: Entry,
    pub merged_from: Vec<i64>,
    pub changed: Vec<String>,
}

pub fn run(conn: &mut Connection, ctx: &Context, args: EditArgs) -> Result<EditReport> {
    let mut changes: Vec<(Field, Option<String>)> = Vec::new();
    if let Some(symptom) = &args.symptom {
        let symptom =
            clean(Some(symptom)).ok_or_else(|| Error::usage("--symptom cannot be empty"))?;
        changes.push((Field::Symptom, Some(symptom)));
    }
    for (field, value) in [
        (Field::Workaround, &args.workaround),
        (Field::Hypothesis, &args.hypothesis),
        (Field::Fix, &args.fix),
        (Field::Ref, &args.reference),
    ] {
        if let Some(value) = value {
            changes.push((field, clean(Some(value))));
        }
    }
    if let Some(subject) = &args.subject {
        changes.push((Field::Subject, normalize_subject(Some(subject))?));
    }
    if let Some(silent) = args.silent {
        changes.push((Field::Silent, Some(silent.to_string())));
    }
    if !args.occurrences.is_empty() && args.cost.is_none() {
        return Err(Error::usage(
            "--occurrence names the occurrences whose cost changes; give --cost too",
        ));
    }
    if changes.is_empty() && args.cost.is_none() {
        return Err(Error::usage(
            "nothing to change: give at least one of --symptom, --workaround, --hypothesis, \
             --fix, --subject, --silent, --no-silent, --ref or --cost",
        ));
    }

    let now = ctx.now_text();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (entry, merged_from) = store::resolve_entry(&tx, args.id)?;
    if entry.status != Status::Open {
        return Err(Error::usage(format!(
            "entry {} is {}; only open entries can be edited (run `snags reopen {}` first)",
            entry.id, entry.status, entry.id
        )));
    }
    let mut changed: Vec<String> = store::update_fields(&tx, &entry, &changes, &now)?
        .into_iter()
        .map(|field| field.column().to_owned())
        .collect();
    if let Some(cost) = args.cost {
        let mut costs_changed = false;
        for occurrence in occurrences_to_reclassify(&tx, entry.id, &args.occurrences)? {
            if let Some(field) = store::set_occurrence_cost(&tx, &occurrence, cost, &now)? {
                changed.push(field);
                costs_changed = true;
            }
        }
        if costs_changed {
            store::touch(&tx, entry.id, &now)?;
        }
    }
    store::reindex(&tx, entry.id)?;
    let (entry, _) = store::resolve_entry(&tx, entry.id)?;
    tx.commit()?;
    Ok(EditReport {
        entry,
        merged_from,
        changed,
    })
}

fn occurrences_to_reclassify(
    conn: &Connection,
    entry_id: i64,
    ids: &[i64],
) -> Result<Vec<Occurrence>> {
    if ids.is_empty() {
        return store::occurrences(conn, entry_id);
    }
    let mut occurrences: Vec<Occurrence> = Vec::new();
    for id in ids {
        if occurrences.iter().any(|occurrence| occurrence.id == *id) {
            continue;
        }
        let occurrence = store::find_occurrence(conn, *id)?
            .ok_or_else(|| Error::NotFound(format!("no occurrence o{id}")))?;
        if occurrence.entry_id != entry_id {
            return Err(Error::usage(format!(
                "o{id} belongs to entry {}, not {entry_id}",
                occurrence.entry_id
            )));
        }
        occurrences.push(occurrence);
    }
    Ok(occurrences)
}

impl Report for EditReport {
    fn text(&self) -> String {
        let mut out = String::new();
        for id in &self.merged_from {
            out.push_str(&format!("{id} was merged into {}.\n", self.entry.id));
        }
        if self.changed.is_empty() {
            out.push_str(&format!(
                "No changes to {}: the values are the same.\n",
                self.entry.id
            ));
        } else {
            out.push_str(&format!(
                "Updated {}: {}. Earlier values: snags show {} --history\n",
                self.entry.id,
                self.changed.join(", "),
                self.entry.id
            ));
        }
        out
    }
}
