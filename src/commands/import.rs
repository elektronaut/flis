use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};
use serde::Serialize;

use super::Context;
use crate::error::{Error, Result};
use crate::import::{self, Malformed};
use crate::model::{Cost, Origin};
use crate::output::{Report, plural};
use crate::store::{self, NewEntry};

pub struct ImportArgs<'a> {
    pub file: &'a Path,
    pub dry_run: bool,
    pub force: bool,
    pub project_map: Option<&'a Path>,
}

#[derive(Debug, Serialize)]
pub struct ProjectCount {
    pub name: Option<String>,
    pub entries: usize,
}

#[derive(Debug, Serialize)]
pub struct PreviousImport {
    pub source: String,
    pub hash: String,
    pub entries: i64,
    pub at: String,
}

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub source: String,
    pub hash: String,
    pub dry_run: bool,
    /// Whether a `---` line ended a header.
    pub header: bool,
    /// Non-blank lines after the header.
    pub lines: usize,
    pub detail_lines: usize,
    /// Entries parsed from the file.
    pub entries: usize,
    /// Entries written; none on a dry run.
    pub imported: usize,
    pub first_id: Option<i64>,
    pub last_id: Option<i64>,
    /// Distinct projects as written in the file.
    pub projects_as_written: usize,
    /// Projects after normalisation and the project map, most entries first.
    pub projects: Vec<ProjectCount>,
    /// Keys of the project map that matched no project.
    pub unused_map_keys: Vec<String>,
    pub malformed: Vec<Malformed>,
    /// Earlier imports of the same content or path.
    pub previous: Vec<PreviousImport>,
}

fn read_project_map(path: &Path) -> Result<HashMap<String, Option<String>>> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        Error::usage(format!(
            "cannot read --project-map {}: {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        Error::usage(format!(
            "--project-map {} must be a JSON object of old name to new name or null: {error}",
            path.display()
        ))
    })
}

pub fn run(conn: &mut Connection, ctx: &Context, args: &ImportArgs<'_>) -> Result<ImportReport> {
    let bytes = std::fs::read(args.file)
        .map_err(|error| Error::usage(format!("cannot read {}: {error}", args.file.display())))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Error::usage(format!("{} is not UTF-8 text", args.file.display())))?;
    let map = args.project_map.map(read_project_map).transpose()?;

    let mut log = import::parse(text);
    let unused_map_keys = match &map {
        Some(map) => import::apply_project_map(&mut log.entries, map),
        None => Vec::new(),
    };
    let source = std::fs::canonicalize(args.file)
        .unwrap_or_else(|_| args.file.to_owned())
        .display()
        .to_string();
    let hash = import::content_hash(&bytes);

    let mut written = Vec::new();
    let tx = conn.transaction_with_behavior(if args.dry_run {
        TransactionBehavior::Deferred
    } else {
        TransactionBehavior::Immediate
    })?;
    let previous = previous_imports(&tx, &source, &hash)?;
    if !args.dry_run {
        if let Some(earlier) = previous.first().filter(|_| !args.force) {
            let what = if earlier.hash == hash {
                "this content"
            } else {
                "a file at this path"
            };
            return Err(Error::usage(format!(
                "{what} was already imported at {} ({}); \
                 run with --force to import it again and duplicate the entries",
                earlier.at,
                plural(earlier.entries.max(0) as u64, "entry", "entries")
            )));
        }
        for entry in &log.entries {
            let at = format!("{}T00:00:00Z", entry.date);
            let new_entry = NewEntry {
                symptom: entry.symptom.clone(),
                workaround: entry.fix.clone(),
                hypothesis: None,
                fix: None,
                subject: None,
                silent: false,
            };
            let id = store::insert_entry(&tx, &new_entry, &at)?;
            let origin = Origin {
                project: entry.project.clone(),
                ..Origin::default()
            };
            store::insert_occurrence(&tx, id, &at, &origin, Cost::Minor, entry.detail.as_deref())?;
            store::reindex(&tx, id)?;
            written.push(id);
        }
        tx.execute(
            "INSERT INTO imports (source, hash, entries, at) VALUES (?1, ?2, ?3, ?4)",
            params![source, hash, written.len() as i64, ctx.now_text()],
        )?;
    }
    tx.commit()?;

    let projects_as_written = log
        .entries
        .iter()
        .map(|entry| entry.project_raw.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let mut counts: BTreeMap<Option<&str>, usize> = BTreeMap::new();
    for entry in &log.entries {
        *counts.entry(entry.project.as_deref()).or_default() += 1;
    }
    let mut projects: Vec<ProjectCount> = counts
        .into_iter()
        .map(|(name, entries)| ProjectCount {
            name: name.map(str::to_owned),
            entries,
        })
        .collect();
    projects.sort_by(|a, b| {
        b.entries
            .cmp(&a.entries)
            .then(a.name.is_none().cmp(&b.name.is_none()))
            .then(a.name.cmp(&b.name))
    });

    Ok(ImportReport {
        source,
        hash,
        dry_run: args.dry_run,
        header: log.header,
        lines: log.lines,
        detail_lines: log.detail_lines,
        entries: log.entries.len(),
        imported: written.len(),
        first_id: written.first().copied(),
        last_id: written.last().copied(),
        projects_as_written,
        projects,
        unused_map_keys,
        malformed: log.malformed,
        previous,
    })
}

fn previous_imports(conn: &Connection, source: &str, hash: &str) -> Result<Vec<PreviousImport>> {
    let mut stmt = conn.prepare(
        "SELECT source, hash, entries, at FROM imports
         WHERE hash = ?1 OR source = ?2 ORDER BY hash = ?1 DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![hash, source], |row| {
        Ok(PreviousImport {
            source: row.get(0)?,
            hash: row.get(1)?,
            entries: row.get(2)?,
            at: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl Report for ImportReport {
    fn text(&self) -> String {
        let mut out = String::new();
        let entries = plural(self.entries as u64, "entry", "entries");
        if self.dry_run {
            let _ = writeln!(
                out,
                "Dry run: would import {entries} from {}. Nothing was written.",
                self.source
            );
        } else {
            let _ = write!(
                out,
                "Imported {}",
                plural(self.imported as u64, "entry", "entries")
            );
            if let (Some(first), Some(last)) = (self.first_id, self.last_id) {
                let _ = write!(out, " (ids {first} to {last})");
            }
            let _ = writeln!(out, " from {}, each open with one occurrence.", self.source);
        }
        if !self.header {
            out.push_str("No `---` line ends a header, so the whole file was read.\n");
        }
        let _ = writeln!(
            out,
            "Read {} after the header: {entries}, {}, {} malformed.",
            plural(self.lines as u64, "line", "lines"),
            plural(self.detail_lines as u64, "detail line", "detail lines"),
            self.malformed.len()
        );

        if !self.projects.is_empty() {
            let _ = writeln!(
                out,
                "\nProjects after normalisation ({} from {} as written):",
                self.projects.len(),
                self.projects_as_written
            );
            let width = self
                .projects
                .iter()
                .map(|p| p.name.as_deref().unwrap_or("(no project)").chars().count())
                .max()
                .unwrap_or(0);
            for project in &self.projects {
                let _ = writeln!(
                    out,
                    "  {:width$}  {}",
                    project.name.as_deref().unwrap_or("(no project)"),
                    project.entries
                );
            }
        }
        if !self.unused_map_keys.is_empty() {
            let _ = writeln!(
                out,
                "\nProject map keys that matched nothing: {}",
                self.unused_map_keys.join(", ")
            );
        }
        if !self.malformed.is_empty() {
            out.push_str(
                "\nMalformed lines, not imported (not four fields separated by ` · `, or no \
                 date or symptom). Add them by hand with `snags add`:\n",
            );
            for malformed in &self.malformed {
                let _ = writeln!(out, "  line {}: {}", malformed.line, malformed.text);
                for detail in &malformed.detail {
                    let _ = writeln!(out, "    {detail}");
                }
            }
        }
        if let Some(earlier) = self.previous.first() {
            let what = if earlier.hash == self.hash {
                "This content"
            } else {
                "A file at this path"
            };
            let _ = writeln!(
                out,
                "\n{what} was imported before, at {} ({}).{}",
                earlier.at,
                plural(earlier.entries.max(0) as u64, "entry", "entries"),
                if self.dry_run {
                    " A real import needs --force."
                } else {
                    ""
                }
            );
        }
        if !self.dry_run && self.imported > 0 {
            out.push_str(
                "\nEvery entry arrives open. A first review classifies them and merges duplicates.\n",
            );
        }
        out
    }
}
