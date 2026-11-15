use std::fmt::Write;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{Context, clean, normalize_subject};
use crate::error::{Error, Result};
use crate::model::{Cost, Entry};
use crate::output::{Report, one_line};
use crate::search::{self, Filter};
use crate::store::{self, NewEntry};

use super::search::Hit;

/// The fields `add` takes, from arguments or from `--input FILE`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    pub symptom: Option<String>,
    pub workaround: Option<String>,
    pub hypothesis: Option<String>,
    pub fix: Option<String>,
    pub subject: Option<String>,
    pub silent: Option<bool>,
    pub cost: Option<Cost>,
    pub note: Option<String>,
    pub project: Option<String>,
    pub agent: Option<String>,
}

impl AddInput {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            Error::usage(format!("cannot read --input {}: {error}", path.display()))
        })?;
        serde_json::from_str(&text).map_err(|error| {
            Error::usage(format!(
                "--input {} is not a valid entry: {error}",
                path.display()
            ))
        })
    }

    /// Fields given here win over the ones in `base`.
    pub fn over(self, base: Self) -> Self {
        Self {
            symptom: self.symptom.or(base.symptom),
            workaround: self.workaround.or(base.workaround),
            hypothesis: self.hypothesis.or(base.hypothesis),
            fix: self.fix.or(base.fix),
            subject: self.subject.or(base.subject),
            silent: self.silent.or(base.silent),
            cost: self.cost.or(base.cost),
            note: self.note.or(base.note),
            project: self.project.or(base.project),
            agent: self.agent.or(base.agent),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SubjectSuggestion {
    pub subject: String,
    /// Existing subjects close to the new one.
    pub closest: Vec<String>,
    /// The most used existing subjects, when none is close.
    pub existing: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AddReport {
    pub id: i64,
    pub occurrence_id: i64,
    pub entry: Entry,
    pub similar: Vec<Hit>,
    pub new_subject: Option<SubjectSuggestion>,
}

pub fn run(conn: &mut Connection, ctx: &Context, input: AddInput) -> Result<AddReport> {
    let symptom = clean(input.symptom.as_deref())
        .ok_or_else(|| Error::usage("a symptom is required: snags add \"<what happened>\""))?;
    let subject = normalize_subject(input.subject.as_deref())?;
    let origin = ctx.origin(input.project.as_deref(), input.agent.as_deref());
    let now = ctx.now_text();

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let known_subjects = store::subjects(&tx)?;
    let new_entry = NewEntry {
        symptom,
        workaround: clean(input.workaround.as_deref()),
        hypothesis: clean(input.hypothesis.as_deref()),
        fix: clean(input.fix.as_deref()),
        subject,
        silent: input.silent.unwrap_or(false),
    };
    let id = store::insert_entry(&tx, &new_entry, &now)?;
    let occurrence_id = store::insert_occurrence(
        &tx,
        id,
        &now,
        &origin,
        input.cost.unwrap_or_default(),
        clean(input.note.as_deref()).as_deref(),
    )?;
    store::reindex(&tx, id)?;

    let tokens = search::tokenize(&format!(
        "{} {}",
        new_entry.symptom,
        new_entry.subject.as_deref().unwrap_or("")
    ));
    let filter = Filter {
        status: None,
        exclude: Some(id),
        limit: 3,
    };
    let (_, similar_ids) = search::search(&tx, &tokens, &filter)?;
    let similar = Hit::load(&tx, &similar_ids)?;
    let entry = store::find_entry(&tx, id)?
        .ok_or_else(|| Error::Internal(format!("entry {id} vanished")))?;
    tx.commit()?;

    let new_subject = new_entry
        .subject
        .filter(|subject| !known_subjects.iter().any(|(known, _)| known == subject))
        .filter(|_| !known_subjects.is_empty())
        .map(|subject| {
            let closest = closest_subjects(&subject, &known_subjects);
            let existing = if closest.is_empty() {
                known_subjects
                    .iter()
                    .take(8)
                    .map(|(name, _)| name.clone())
                    .collect()
            } else {
                Vec::new()
            };
            SubjectSuggestion {
                subject,
                closest,
                existing,
            }
        });

    Ok(AddReport {
        id,
        occurrence_id,
        entry,
        similar,
        new_subject,
    })
}

/// Up to three known subjects within two edits of `subject`, or sharing a
/// prefix with it, nearest first and the more used first on a tie.
pub fn closest_subjects(subject: &str, known: &[(String, i64)]) -> Vec<String> {
    let mut ranked: Vec<(usize, i64, &str)> = known
        .iter()
        .filter_map(|(name, count)| {
            let mut distance = levenshtein(subject, name);
            if name.starts_with(subject) || subject.starts_with(name.as_str()) {
                distance = distance.min(1);
            }
            (distance <= 2 && distance < subject.chars().count().max(name.chars().count()))
                .then_some((distance, -count, name.as_str()))
        })
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .take(3)
        .map(|(_, _, name)| name.to_owned())
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != *cb);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

impl Report for AddReport {
    fn text(&self) -> String {
        let mut out = format!("Added {} (o{}).\n", self.id, self.occurrence_id);
        if !self.similar.is_empty() {
            out.push_str(
                "\nSimilar entries. If one is the same problem, it will be merged in review; \
                 next time run `snags seen <id>` instead of adding:\n",
            );
            for hit in &self.similar {
                let _ = writeln!(out, "  {}", hit.heading());
                let _ = writeln!(out, "    {}", one_line(&hit.entry.symptom, 120));
            }
        }
        if let Some(suggestion) = &self.new_subject {
            let _ = write!(out, "\nSubject {:?} is new.", suggestion.subject);
            if let Some(first) = suggestion.closest.first() {
                let _ = writeln!(
                    out,
                    " Closest existing subjects: {}. To switch: snags edit {} --subject {first}",
                    suggestion.closest.join(", "),
                    self.id,
                );
            } else {
                let _ = writeln!(
                    out,
                    " None of the existing subjects is close ({}).",
                    suggestion.existing.join(", ")
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_near_subjects_first() {
        let known = vec![
            ("bundler".to_owned(), 4),
            ("gh".to_owned(), 9),
            ("sandbox".to_owned(), 2),
        ];
        assert_eq!(closest_subjects("bundle", &known)[0], "bundler");
        assert_eq!(closest_subjects("sandbx", &known)[0], "sandbox");
        assert_eq!(closest_subjects("ghx", &known)[0], "gh");
        assert!(closest_subjects("docker", &known).is_empty());
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
