use std::collections::BTreeSet;
use std::fmt::Write;

use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::model::{Entry, HistoryRecord, Occurrence};
use crate::output::{Report, entry_heading, field, plural};
use crate::store;

#[derive(Debug, Serialize)]
pub struct ShowReport {
    pub entry: Entry,
    pub merged_from: Vec<i64>,
    pub occurrences: Vec<Occurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<HistoryRecord>>,
}

pub fn run(conn: &mut Connection, id: i64, with_history: bool) -> Result<ShowReport> {
    let tx = conn.transaction()?;
    let (entry, merged_from) = store::resolve_entry(&tx, id)?;
    let occurrences = store::occurrences(&tx, entry.id)?;
    let history = with_history
        .then(|| store::history(&tx, entry.id))
        .transpose()?;
    tx.commit()?;
    Ok(ShowReport {
        entry,
        merged_from,
        occurrences,
        history,
    })
}

impl Report for ShowReport {
    fn text(&self) -> String {
        let entry = &self.entry;
        let mut out = String::new();
        for id in &self.merged_from {
            let _ = writeln!(out, "{id} was merged into {}.", entry.id);
        }
        let _ = writeln!(out, "{}", entry_heading(entry));
        field(&mut out, "", "symptom", &entry.symptom);
        let optional = [
            ("workaround", &entry.workaround),
            ("hypothesis", &entry.hypothesis),
            ("fix", &entry.fix),
            ("ref", &entry.reference),
        ];
        for (label, value) in optional {
            if let Some(value) = value {
                field(&mut out, "", label, value);
            }
        }
        if let Some(location) = entry.fix_location {
            field(&mut out, "", "fixed in", location.as_str());
        }
        if let Some(note) = &entry.resolution_note {
            field(&mut out, "", "resolution", note);
        }
        if let Some(at) = &entry.resolved_at {
            field(&mut out, "", "resolved", at);
        }
        if entry.regressions > 0 {
            field(&mut out, "", "regressions", &entry.regressions.to_string());
        }
        let _ = writeln!(
            out,
            "created {} · updated {}",
            entry.created_at, entry.updated_at
        );

        let projects: BTreeSet<_> = self
            .occurrences
            .iter()
            .map(|o| o.project.as_deref())
            .collect();
        let _ = writeln!(
            out,
            "\n{}, {}:",
            plural(self.occurrences.len() as u64, "occurrence", "occurrences"),
            plural(projects.len() as u64, "project", "projects")
        );
        for occurrence in &self.occurrences {
            let mut place = occurrence.project.clone().unwrap_or_else(|| "-".to_owned());
            if let Some(branch) = &occurrence.branch {
                let _ = write!(place, "@{branch}");
            }
            if let Some(cwd) = occurrence.cwd.as_deref().filter(|cwd| *cwd != ".") {
                let _ = write!(place, " {cwd}");
            }
            let _ = writeln!(
                out,
                "o{} {} {} {} {}",
                occurrence.id,
                occurrence.at,
                occurrence.cost,
                place,
                occurrence.agent.as_deref().unwrap_or("-"),
            );
            if let Some(note) = &occurrence.note {
                field(&mut out, "  ", "note", note);
            }
        }

        if let Some(history) = &self.history {
            if history.is_empty() {
                out.push_str("\nNo earlier values.\n");
            } else {
                out.push_str("\nEarlier values (replaced at):\n");
                for record in history {
                    let label = format!("{} {}", record.at, record.field);
                    field(
                        &mut out,
                        "",
                        &label,
                        record.old_value.as_deref().unwrap_or("(empty)"),
                    );
                }
            }
        }
        out
    }
}
