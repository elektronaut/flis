use std::collections::HashMap;
use std::fmt::Write;

use clap::ValueEnum;
use rusqlite::Connection;
use serde::Serialize;

use super::Context;
use crate::clock::{date_part, format_timestamp, parse_since};
use crate::error::{Error, Result};
use crate::mask::mask;
use crate::model::{Entry, Occurrence, Status};
use crate::output::{Report, entry_heading, plural};
use crate::rank::{self, Score};
use crate::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Markdown, one section per entry
    #[default]
    Md,
    /// One JSON object, the same as --json
    Json,
}

pub struct ExportArgs {
    pub format: Format,
    pub status: Option<Status>,
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ExportEntry {
    #[serde(flatten)]
    pub entry: Entry,
    /// The ranking factors over the occurrences inside the window.
    pub rank: Score,
    /// Open with a `ref`: reported upstream, waiting on a fix.
    pub waiting: bool,
    pub occurrences: Vec<Occurrence>,
    #[serde(skip)]
    masked: usize,
}

#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub generated_at: String,
    pub status: Option<Status>,
    /// Ranking counts occurrences since then, as `list` does by default.
    pub since: String,
    pub max_tokens: Option<usize>,
    /// The size of the output at four characters per token.
    pub estimated_tokens: usize,
    pub total: usize,
    pub included: usize,
    /// Entries left out to stay within `max_tokens`.
    pub omitted: usize,
    /// Spans replaced by the secret masking.
    pub masked: usize,
    pub entries: Vec<ExportEntry>,
    #[serde(skip)]
    format: Format,
    #[serde(skip)]
    first_entry_tokens: Option<usize>,
}

pub fn run(conn: &mut Connection, ctx: &Context, args: &ExportArgs) -> Result<ExportReport> {
    if args.max_tokens == Some(0) {
        return Err(Error::usage("--max-tokens must be at least 1"));
    }
    let since = format_timestamp(parse_since("90d", ctx.now)?);
    let tx = conn.transaction()?;
    let entries = store::all_live_entries(&tx)?;
    let occurrences = store::occurrences_since(&tx, "")?;
    tx.commit()?;

    let mut by_entry: HashMap<i64, Vec<Occurrence>> = HashMap::new();
    for occurrence in occurrences {
        by_entry
            .entry(occurrence.entry_id)
            .or_default()
            .push(occurrence);
    }

    let mut ranked: Vec<(bool, Score, ExportEntry)> = Vec::new();
    for entry in entries {
        if args.status.is_some_and(|status| entry.status != status) {
            continue;
        }
        let occurrences = by_entry.remove(&entry.id).unwrap_or_default();
        let window = rank::score(occurrences.iter().filter(|o| o.at >= since));
        let in_window = window.occurrences > 0;
        let order = if in_window {
            window.clone()
        } else {
            rank::score(&occurrences)
        };
        let waiting = entry.is_waiting();
        let mut masked = 0;
        let entry = mask_entry(entry, &mut masked);
        let occurrences = occurrences
            .into_iter()
            .map(|o| mask_occurrence(o, &mut masked))
            .collect();
        let item = ExportEntry {
            entry,
            rank: window,
            waiting,
            occurrences,
            masked,
        };
        ranked.push((in_window, order, item));
    }
    ranked.sort_by(|a, b| {
        a.2.waiting
            .cmp(&b.2.waiting)
            .then(b.0.cmp(&a.0))
            .then_with(|| {
                rank::by_rank(
                    (a.2.entry.silent, &a.1, a.2.entry.id),
                    (b.2.entry.silent, &b.1, b.2.entry.id),
                )
            })
    });

    let total = ranked.len();
    let mut report = ExportReport {
        generated_at: ctx.now_text(),
        status: args.status,
        since,
        max_tokens: args.max_tokens,
        estimated_tokens: args.max_tokens.unwrap_or(0),
        total,
        included: total,
        omitted: total,
        masked: ranked.iter().map(|(_, _, item)| item.masked).sum(),
        entries: Vec::new(),
        format: args.format,
        first_entry_tokens: None,
    };
    let header = report.envelope_size();
    let mut size = header;
    for (_, _, item) in ranked {
        let item_size = report.entry_size(&item);
        if let Some(max) = args.max_tokens
            && tokens(size + item_size) > max
        {
            if report.entries.is_empty() {
                report.first_entry_tokens = Some(tokens(header + item_size));
            }
            break;
        }
        size += item_size;
        report.entries.push(item);
    }
    report.included = report.entries.len();
    report.omitted = total - report.included;
    report.masked = report.entries.iter().map(|item| item.masked).sum();
    report.estimated_tokens = tokens(report.rendered_size());
    Ok(report)
}

fn tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}

fn mask_text(text: &mut String, count: &mut usize) {
    let (masked, spans) = mask(text);
    if spans > 0 {
        *text = masked;
        *count += spans;
    }
}

fn mask_option(text: &mut Option<String>, count: &mut usize) {
    if let Some(text) = text {
        mask_text(text, count);
    }
}

fn mask_entry(mut entry: Entry, count: &mut usize) -> Entry {
    mask_text(&mut entry.symptom, count);
    for text in [
        &mut entry.workaround,
        &mut entry.hypothesis,
        &mut entry.fix,
        &mut entry.subject,
        &mut entry.reference,
        &mut entry.resolution_note,
    ] {
        mask_option(text, count);
    }
    entry
}

fn mask_occurrence(mut occurrence: Occurrence, count: &mut usize) -> Occurrence {
    for text in [
        &mut occurrence.project,
        &mut occurrence.branch,
        &mut occurrence.cwd,
        &mut occurrence.agent,
        &mut occurrence.session,
        &mut occurrence.note,
    ] {
        mask_option(text, count);
    }
    occurrence
}

impl ExportReport {
    fn envelope_size(&self) -> usize {
        match self.format {
            Format::Json => serde_json::to_string(self).map_or(0, |json| json.len() + 1),
            Format::Md => {
                let footer = footer(Some(usize::MAX), 0)
                    .len()
                    .max(footer(None, self.total).len());
                self.header_text(true).len() + footer
            }
        }
    }

    fn entry_size(&self, item: &ExportEntry) -> usize {
        match self.format {
            Format::Json => serde_json::to_string(item).map_or(0, |json| json.len() + 1),
            Format::Md => entry_markdown(item).len(),
        }
    }

    fn rendered_size(&self) -> usize {
        match self.format {
            Format::Json => serde_json::to_string(self).map_or(0, |json| json.len() + 1),
            Format::Md => self.text().len(),
        }
    }

    /// With `longest`, the header as if every entry were left out.
    fn header_text(&self, longest: bool) -> String {
        let omitted = if longest { self.total } else { self.omitted };
        let included = if longest { self.total } else { self.included };
        let status = self.status.map_or("all", Status::as_str);
        let mut out = format!(
            "# flis export\n\n{} {} entries, generated {}, ranked silent first, then by score over \
             occurrences since {}; reported, waiting entries last.",
            if omitted > 0 {
                format!("{included} of {}", self.total)
            } else {
                self.total.to_string()
            },
            status,
            self.generated_at,
            date_part(&self.since),
        );
        if omitted > 0 {
            let _ = write!(
                out,
                " {} left out to stay within --max-tokens {}.",
                plural(omitted as u64, "entry", "entries"),
                self.max_tokens.unwrap_or(0)
            );
        }
        let _ = writeln!(
            out,
            " {} masked in this output; the database is unchanged.",
            plural(self.masked as u64, "secret", "secrets")
        );
        out
    }
}

fn push_field(out: &mut String, label: &str, text: &str) {
    let mut lines = text.trim_end().lines();
    let _ = writeln!(out, "- {label}: {}", lines.next().unwrap_or(""));
    for line in lines {
        let _ = writeln!(out, "  {line}");
    }
}

fn entry_markdown(item: &ExportEntry) -> String {
    let entry = &item.entry;
    let rank = &item.rank;
    let mut out = format!("\n## {}\n\n", entry_heading(entry));
    if rank.occurrences > 0 {
        let _ = writeln!(
            out,
            "Score {} = weight {} × {} × {} in the window.\n",
            rank.score,
            rank.weight,
            plural(rank.days, "day", "days"),
            plural(rank.projects, "project", "projects"),
        );
    } else {
        out.push_str("Not seen in the window.\n\n");
    }
    push_field(&mut out, "symptom", &entry.symptom);
    let optional = [
        ("workaround", &entry.workaround),
        ("hypothesis", &entry.hypothesis),
        ("fix", &entry.fix),
        ("ref", &entry.reference),
    ];
    for (label, value) in optional {
        if let Some(value) = value {
            push_field(&mut out, label, value);
        }
    }
    if item.waiting {
        push_field(&mut out, "reported", "waiting on the ref");
    }
    if let Some(location) = entry.fix_location {
        push_field(&mut out, "fixed in", location.as_str());
    }
    if let Some(note) = &entry.resolution_note {
        let label = if entry.status == Status::Accepted {
            "accepted"
        } else {
            "resolution"
        };
        push_field(&mut out, label, note);
    }
    if let Some(at) = &entry.resolved_at {
        push_field(&mut out, "resolved", at);
    }
    if entry.regressions > 0 {
        push_field(&mut out, "regressions", &entry.regressions.to_string());
    }
    push_field(
        &mut out,
        "created",
        &format!("{} · updated {}", entry.created_at, entry.updated_at),
    );

    let _ = writeln!(
        out,
        "\n{}:\n",
        plural(item.occurrences.len() as u64, "occurrence", "occurrences")
    );
    for occurrence in &item.occurrences {
        let mut line = format!(
            "- o{} · {} · {} · {}",
            occurrence.id,
            occurrence.at,
            occurrence.cost,
            occurrence.project.as_deref().unwrap_or("-")
        );
        if let Some(branch) = &occurrence.branch {
            let _ = write!(line, "@{branch}");
        }
        if let Some(cwd) = occurrence.cwd.as_deref().filter(|cwd| *cwd != ".") {
            let _ = write!(line, " {cwd}");
        }
        let _ = write!(line, " · {}", occurrence.agent.as_deref().unwrap_or("-"));
        if let Some(session) = &occurrence.session {
            let _ = write!(line, " · session {session}");
        }
        let _ = writeln!(out, "{line}");
        if let Some(note) = &occurrence.note {
            for text in note.trim_end().lines() {
                let _ = writeln!(out, "  {text}");
            }
        }
    }
    out
}

impl Report for ExportReport {
    fn text(&self) -> String {
        let mut out = self.header_text(false);
        for item in &self.entries {
            out.push_str(&entry_markdown(item));
        }
        out.push_str(&footer(self.first_entry_tokens, self.omitted));
        out
    }
}

fn footer(first_entry_tokens: Option<usize>, omitted: usize) -> String {
    if let Some(needed) = first_entry_tokens {
        format!("\nThe first entry alone needs about {needed} tokens; raise --max-tokens.\n")
    } else if omitted > 0 {
        format!(
            "\n{} left out; raise --max-tokens or narrow with --status.\n",
            plural(omitted as u64, "more entry", "more entries")
        )
    } else {
        String::new()
    }
}
