use std::collections::HashMap;
use std::fmt::Write;

use clap::ValueEnum;
use rusqlite::Connection;
use serde::Serialize;

use super::{Context, clean};
use crate::clock::{date_part, format_timestamp, parse_since};
use crate::error::Result;
use crate::model::{Entry, Occurrence, Status};
use crate::output::{Report, entry_heading, one_line, plural};
use crate::rank::{self, Score};
use crate::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// Silent first, then by score, then the most recently seen
    #[default]
    Rank,
    /// The most recently seen first
    Recent,
}

pub struct ListArgs<'a> {
    pub status: Option<Status>,
    pub project: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub since: &'a str,
    pub sort: Sort,
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct Ranked {
    #[serde(flatten)]
    pub entry: Entry,
    #[serde(flatten)]
    pub score: Score,
}

#[derive(Debug, Serialize)]
pub struct Group {
    pub entries: Vec<Ranked>,
    /// Entries left out by `--limit`.
    pub more: usize,
}

#[derive(Debug, Serialize)]
pub struct ListReport {
    pub status: Option<Status>,
    pub since: String,
    pub sort: Sort,
    pub ranked: Group,
    /// Open entries with a `ref`: reported upstream, waiting on a fix.
    pub waiting: Group,
    /// Entries that match the filters but were not seen inside the window.
    pub not_seen_since: usize,
}

pub fn run(conn: &mut Connection, ctx: &Context, args: &ListArgs<'_>) -> Result<ListReport> {
    let since = format_timestamp(parse_since(args.since, ctx.now)?);
    let project = clean(args.project);
    let subject = clean(args.subject).map(|subject| subject.to_lowercase());

    let tx = conn.transaction()?;
    let entries = store::all_live_entries(&tx)?;
    let occurrences = store::occurrences_since(&tx, "")?;
    tx.commit()?;

    let mut by_entry: HashMap<i64, Vec<&Occurrence>> = HashMap::new();
    for occurrence in &occurrences {
        by_entry
            .entry(occurrence.entry_id)
            .or_default()
            .push(occurrence);
    }

    let mut ranked = Vec::new();
    let mut waiting = Vec::new();
    let mut not_seen_since = 0;
    for entry in entries {
        if args.status.is_some_and(|status| entry.status != status)
            || subject
                .as_ref()
                .is_some_and(|subject| entry.subject.as_ref() != Some(subject))
        {
            continue;
        }
        let all = by_entry
            .get(&entry.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if let Some(project) = &project
            && !all.iter().any(|o| o.project.as_ref() == Some(project))
        {
            continue;
        }
        let in_window: Vec<&Occurrence> = all.iter().copied().filter(|o| o.at >= since).collect();
        let in_project = project.as_ref().is_none_or(|project| {
            in_window
                .iter()
                .any(|o| o.project.as_ref() == Some(project))
        });
        if in_window.is_empty() || !in_project {
            not_seen_since += 1;
            continue;
        }
        let score = rank::score(in_window);
        let item = Ranked { entry, score };
        if item.entry.is_waiting() {
            waiting.push(item);
        } else {
            ranked.push(item);
        }
    }

    Ok(ListReport {
        status: args.status,
        since,
        sort: args.sort,
        ranked: group(ranked, args.sort, args.limit),
        waiting: group(waiting, args.sort, args.limit),
        not_seen_since,
    })
}

fn group(mut entries: Vec<Ranked>, sort: Sort, limit: usize) -> Group {
    match sort {
        Sort::Rank => entries.sort_by(|a, b| {
            rank::by_rank(
                (a.entry.silent, &a.score, a.entry.id),
                (b.entry.silent, &b.score, b.entry.id),
            )
        }),
        Sort::Recent => {
            entries.sort_by(|a, b| rank::by_recent((&a.score, a.entry.id), (&b.score, b.entry.id)));
        }
    }
    let more = entries.len().saturating_sub(limit);
    entries.truncate(limit);
    Group { entries, more }
}

fn write_group(out: &mut String, group: &Group) {
    for item in &group.entries {
        let score = &item.score;
        let _ = writeln!(
            out,
            "{} · score {} = weight {} × {} × {} · {}, last {}",
            entry_heading(&item.entry),
            score.score,
            score.weight,
            plural(score.days, "day", "days"),
            plural(score.projects, "project", "projects"),
            plural(score.occurrences, "occurrence", "occurrences"),
            score.last_seen.as_deref().map_or("-", date_part),
        );
        let _ = writeln!(out, "  {}", one_line(&item.entry.symptom, 140));
        if item.entry.is_waiting()
            && let Some(reference) = &item.entry.reference
        {
            let _ = writeln!(out, "  ref: {reference}");
        }
    }
    if group.more > 0 {
        let _ = writeln!(out, "… and {} more; raise --limit to see them.", group.more);
    }
}

impl Report for ListReport {
    fn text(&self) -> String {
        let status = self.status.map_or("All", |status| match status {
            Status::Open => "Open",
            Status::Resolved => "Resolved",
            Status::Accepted => "Accepted",
        });
        let order = match self.sort {
            Sort::Rank => "ranked silent first, then by score",
            Sort::Recent => "most recently seen first",
        };
        let mut out = format!(
            "{status} entries seen since {}, {order}. Only occurrences since then count.\n",
            date_part(&self.since)
        );
        if self.ranked.entries.is_empty() && self.waiting.entries.is_empty() {
            out.push_str("None.\n");
        }
        if !self.ranked.entries.is_empty() {
            out.push('\n');
            write_group(&mut out, &self.ranked);
        }
        if !self.waiting.entries.is_empty() {
            out.push_str("\nReported, waiting (open with a ref):\n");
            write_group(&mut out, &self.waiting);
        }
        if self.not_seen_since > 0 {
            let _ = writeln!(
                out,
                "\n{} not seen since {}; widen with --since.",
                plural(self.not_seen_since as u64, "more entry", "more entries"),
                date_part(&self.since)
            );
        }
        out
    }
}
