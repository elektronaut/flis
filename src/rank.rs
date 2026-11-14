use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::Serialize;

use crate::clock::date_part;
use crate::model::Occurrence;

/// The three ranking factors over the occurrences in the window, and their
/// product.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Score {
    pub score: u64,
    /// Occurrences weighted by cost: minor 1, major 3, blocker 5.
    pub weight: u64,
    /// Distinct UTC days with an occurrence.
    pub days: u64,
    /// Distinct projects, with occurrences outside any repository counted as one.
    pub projects: u64,
    pub occurrences: u64,
    pub last_seen: Option<String>,
}

pub fn score<'a>(occurrences: impl IntoIterator<Item = &'a Occurrence>) -> Score {
    let mut weight = 0;
    let mut count = 0;
    let mut days = BTreeSet::new();
    let mut projects = BTreeSet::new();
    let mut last_seen: Option<&str> = None;
    for occurrence in occurrences {
        weight += occurrence.cost.weight();
        count += 1;
        days.insert(date_part(&occurrence.at));
        projects.insert(occurrence.project.as_deref());
        if last_seen.is_none_or(|last| occurrence.at.as_str() > last) {
            last_seen = Some(&occurrence.at);
        }
    }
    let days = days.len() as u64;
    let projects = projects.len() as u64;
    Score {
        score: weight * days * projects,
        weight,
        days,
        projects,
        occurrences: count,
        last_seen: last_seen.map(str::to_owned),
    }
}

/// Silent entries first, then by score, then the most recently seen.
pub fn by_rank(a: (bool, &Score, i64), b: (bool, &Score, i64)) -> Ordering {
    b.0.cmp(&a.0)
        .then(b.1.score.cmp(&a.1.score))
        .then(b.1.last_seen.cmp(&a.1.last_seen))
        .then(b.2.cmp(&a.2))
}

pub fn by_recent(a: (&Score, i64), b: (&Score, i64)) -> Ordering {
    b.0.last_seen.cmp(&a.0.last_seen).then(b.1.cmp(&a.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Cost;

    fn occurrence(at: &str, project: Option<&str>, cost: Cost) -> Occurrence {
        Occurrence {
            id: 0,
            entry_id: 1,
            at: at.to_owned(),
            project: project.map(str::to_owned),
            branch: None,
            cwd: None,
            agent: None,
            session: None,
            cost,
            note: None,
        }
    }

    #[test]
    fn multiplies_weight_days_and_projects() {
        let list = [
            occurrence("2026-11-01T10:00:00Z", Some("acme"), Cost::Minor),
            occurrence("2026-11-01T11:00:00Z", Some("nova"), Cost::Major),
            occurrence("2026-11-03T09:00:00Z", None, Cost::Blocker),
        ];
        let score = score(&list);
        assert_eq!((score.weight, score.days, score.projects), (9, 2, 3));
        assert_eq!(score.score, 54);
        assert_eq!(score.last_seen.as_deref(), Some("2026-11-03T09:00:00Z"));
    }

    #[test]
    fn silent_beats_score_and_recency_breaks_ties() {
        let high = Score {
            score: 50,
            last_seen: Some("2026-11-01".into()),
            ..Score::default()
        };
        let low = Score {
            score: 1,
            last_seen: Some("2026-10-01".into()),
            ..Score::default()
        };
        let tie_new = Score {
            score: 50,
            last_seen: Some("2026-11-05".into()),
            ..Score::default()
        };
        assert_eq!(by_rank((true, &low, 1), (false, &high, 2)), Ordering::Less);
        assert_eq!(by_rank((false, &high, 1), (false, &low, 2)), Ordering::Less);
        assert_eq!(
            by_rank((false, &tie_new, 1), (false, &high, 2)),
            Ordering::Less
        );
    }
}
