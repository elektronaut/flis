use std::fmt::Write;

use rusqlite::Connection;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::{Entry, Status};
use crate::output::{Report, entry_heading, field, one_line, resolution_text, seen_text};
use crate::search::{self, Filter, Match};
use crate::store::{self, Seen};

#[derive(Debug, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub entry: Entry,
    #[serde(flatten)]
    pub seen: Seen,
}

impl Hit {
    pub fn load(conn: &Connection, ids: &[i64]) -> Result<Vec<Self>> {
        store::entries_by_ids(conn, ids)?
            .into_iter()
            .map(|entry| {
                Ok(Self {
                    seen: store::seen_summary(conn, entry.id)?,
                    entry,
                })
            })
            .collect()
    }

    /// `4 open · bundler · seen 3 times, last 2026-11-14`
    pub fn heading(&self) -> String {
        format!("{} · {}", entry_heading(&self.entry), seen_text(&self.seen))
    }
}

#[derive(Debug, Serialize)]
pub struct SearchReport {
    pub tokens: Vec<String>,
    #[serde(rename = "match")]
    pub matched: Match,
    pub hits: Vec<Hit>,
}

pub fn run(
    conn: &Connection,
    text: &str,
    status: Option<Status>,
    limit: usize,
) -> Result<SearchReport> {
    let tokens = search::tokenize(text);
    if tokens.is_empty() {
        return Err(Error::usage(
            "nothing to search for: give some words from the error or symptom",
        ));
    }
    let filter = Filter {
        status: status.map(Status::as_str),
        exclude: None,
        limit,
    };
    let (matched, ids) = search::search(conn, &tokens, &filter)?;
    Ok(SearchReport {
        tokens,
        matched,
        hits: Hit::load(conn, &ids)?,
    })
}

impl Report for SearchReport {
    fn text(&self) -> String {
        let count = self.hits.len();
        let mut out = match self.matched {
            Match::None => return "No entries match.\n".to_owned(),
            Match::All => format!("{count} matching all words:\n"),
            Match::Any => format!("No entry matches all words. {count} matching some of them:\n"),
        };
        for hit in &self.hits {
            let entry = &hit.entry;
            let _ = writeln!(out, "\n{}", hit.heading());
            field(&mut out, "  ", "symptom", &one_line(&entry.symptom, 300));
            match &entry.workaround {
                Some(workaround) => field(&mut out, "  ", "workaround", workaround),
                None => out.push_str("  workaround: none recorded\n"),
            }
            if entry.status == Status::Resolved {
                let fixed = resolution_text(entry).unwrap_or_else(|| "no details".to_owned());
                field(&mut out, "  ", "fixed", &fixed);
            } else if entry.status == Status::Accepted
                && let Some(note) = &entry.resolution_note
            {
                field(&mut out, "  ", "accepted", note);
            } else if let Some(reference) = &entry.reference {
                field(&mut out, "  ", "reported", reference);
            }
        }
        out
    }
}
