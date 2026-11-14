use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use rusqlite::Connection;
use serde::Serialize;

use super::Context;
use crate::clock::{date_part, format_timestamp, parse_since};
use crate::error::Result;
use crate::model::{Occurrence, Status};
use crate::output::{Report, plural};
use crate::store;

#[derive(Debug, Default, Serialize)]
pub struct StatusCounts {
    pub open: usize,
    pub resolved: usize,
    pub accepted: usize,
}

#[derive(Debug, Serialize)]
pub struct Count {
    pub name: Option<String>,
    pub entries: usize,
    pub occurrences: usize,
}

#[derive(Debug, Serialize)]
pub struct StatsReport {
    pub since: Option<String>,
    pub entries: usize,
    pub occurrences: usize,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
    pub status: StatusCounts,
    /// Open entries with a `ref`: reported upstream, waiting on a fix.
    pub waiting: usize,
    pub silent: usize,
    pub regressions: i64,
    pub projects: Vec<Count>,
    pub subjects: Vec<Count>,
}

pub fn run(conn: &mut Connection, ctx: &Context, since: Option<&str>) -> Result<StatsReport> {
    let since = since
        .map(|raw| parse_since(raw, ctx.now).map(format_timestamp))
        .transpose()?;
    let tx = conn.transaction()?;
    let entries = store::all_live_entries(&tx)?;
    let occurrences = store::occurrences_since(&tx, since.as_deref().unwrap_or(""))?;
    tx.commit()?;

    let mut by_entry: HashMap<i64, Vec<&Occurrence>> = HashMap::new();
    for occurrence in &occurrences {
        by_entry
            .entry(occurrence.entry_id)
            .or_default()
            .push(occurrence);
    }

    let mut report = StatsReport {
        since,
        entries: 0,
        occurrences: 0,
        first_seen: None,
        last_seen: None,
        status: StatusCounts::default(),
        waiting: 0,
        silent: 0,
        regressions: 0,
        projects: Vec::new(),
        subjects: Vec::new(),
    };
    let mut projects: BTreeMap<Option<&str>, (usize, usize)> = BTreeMap::new();
    let mut subjects: BTreeMap<Option<&str>, (usize, usize)> = BTreeMap::new();
    for entry in &entries {
        let seen = by_entry
            .get(&entry.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if report.since.is_some() && seen.is_empty() {
            continue;
        }
        report.entries += 1;
        report.occurrences += seen.len();
        match entry.status {
            Status::Open => report.status.open += 1,
            Status::Resolved => report.status.resolved += 1,
            Status::Accepted => report.status.accepted += 1,
        }
        report.waiting += usize::from(entry.is_waiting());
        report.silent += usize::from(entry.silent);
        report.regressions += entry.regressions;

        let subject = subjects.entry(entry.subject.as_deref()).or_default();
        subject.0 += 1;
        subject.1 += seen.len();

        let mut entry_projects: Vec<Option<&str>> = Vec::new();
        for occurrence in seen {
            let project = occurrence.project.as_deref();
            projects.entry(project).or_default().1 += 1;
            if !entry_projects.contains(&project) {
                entry_projects.push(project);
                projects.entry(project).or_default().0 += 1;
            }
            let at = Some(occurrence.at.as_str());
            if report.first_seen.is_none() || at < report.first_seen.as_deref() {
                report.first_seen = at.map(str::to_owned);
            }
            if at > report.last_seen.as_deref() {
                report.last_seen = at.map(str::to_owned);
            }
        }
    }
    report.projects = counts(projects, |c| c.occurrences);
    report.subjects = counts(subjects, |c| c.entries);
    Ok(report)
}

/// Largest first by `key`, then by the other count, then by name.
fn counts(map: BTreeMap<Option<&str>, (usize, usize)>, key: fn(&Count) -> usize) -> Vec<Count> {
    let mut list: Vec<Count> = map
        .into_iter()
        .map(|(name, (entries, occurrences))| Count {
            name: name.map(str::to_owned),
            entries,
            occurrences,
        })
        .collect();
    list.sort_by(|a, b| {
        key(b)
            .cmp(&key(a))
            .then((b.entries + b.occurrences).cmp(&(a.entries + a.occurrences)))
            .then(a.name.is_none().cmp(&b.name.is_none()))
            .then(a.name.cmp(&b.name))
    });
    list
}

fn write_counts(out: &mut String, list: &[Count], none: &str) {
    let width = list
        .iter()
        .map(|c| c.name.as_deref().unwrap_or(none).chars().count())
        .max()
        .unwrap_or(0);
    for count in list {
        let _ = writeln!(
            out,
            "  {:width$}  {} · {}",
            count.name.as_deref().unwrap_or(none),
            plural(count.entries as u64, "entry", "entries"),
            plural(count.occurrences as u64, "occurrence", "occurrences"),
        );
    }
}

impl Report for StatsReport {
    fn text(&self) -> String {
        let mut out = match &self.since {
            Some(since) => format!(
                "Entries seen since {}; only occurrences since then count.\n",
                date_part(since)
            ),
            None => "All entries.\n".to_owned(),
        };
        let _ = write!(
            out,
            "{}, {}",
            plural(self.entries as u64, "entry", "entries"),
            plural(self.occurrences as u64, "occurrence", "occurrences")
        );
        if let (Some(first), Some(last)) = (&self.first_seen, &self.last_seen) {
            let _ = write!(out, ", {} to {}", date_part(first), date_part(last));
        }
        out.push_str(".\n");
        let _ = writeln!(
            out,
            "Status: {} open ({} reported, waiting), {} resolved, {} accepted.",
            self.status.open, self.waiting, self.status.resolved, self.status.accepted
        );
        let _ = writeln!(
            out,
            "Silent: {}. Regressions: {}.",
            self.silent, self.regressions
        );
        if !self.projects.is_empty() {
            out.push_str("\nProjects, by occurrences:\n");
            write_counts(&mut out, &self.projects, "(no project)");
        }
        if !self.subjects.is_empty() {
            out.push_str("\nSubjects, by entries:\n");
            write_counts(&mut out, &self.subjects, "(no subject)");
        }
        out
    }
}
