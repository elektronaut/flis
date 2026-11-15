use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::{Context, clean, normalize_subject};
use crate::error::{Error, Result};
use crate::model::{Entry, Status};
use crate::output::Report;
use crate::store::{self, Field};

/// The fields to change. `Some("")` clears an optional field.
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
}

#[derive(Debug, Serialize)]
pub struct EditReport {
    pub entry: Entry,
    pub merged_from: Vec<i64>,
    pub changed: Vec<&'static str>,
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
    if changes.is_empty() {
        return Err(Error::usage(
            "nothing to change: give at least one of --symptom, --workaround, --hypothesis, \
             --fix, --subject, --silent, --no-silent or --ref",
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
    let changed = store::update_fields(&tx, &entry, &changes, &now)?;
    store::reindex(&tx, entry.id)?;
    let (entry, _) = store::resolve_entry(&tx, entry.id)?;
    tx.commit()?;
    Ok(EditReport {
        entry,
        merged_from,
        changed: changed.into_iter().map(Field::column).collect(),
    })
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
