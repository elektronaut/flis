use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Match {
    /// Every token matched.
    All,
    /// Nothing matched every token; these match at least one.
    Any,
    /// Nothing matched.
    None,
}

/// Splits free text into the words the FTS5 tokenizer would see: runs of
/// letters and digits, lowercased, without duplicates.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let word = word.to_lowercase();
        if !tokens.contains(&word) {
            tokens.push(word);
        }
        if tokens.len() == 64 {
            break;
        }
    }
    tokens
}

/// An FTS5 query that matches the tokens literally, joined by `AND` or `OR`.
pub fn fts_query(tokens: &[String], operator: &str) -> String {
    tokens
        .iter()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(&format!(" {operator} "))
}

pub struct Filter<'a> {
    pub status: Option<&'a str>,
    pub exclude: Option<i64>,
    pub limit: usize,
}

/// Entry ids matching the tokens, best first: all tokens if anything matches
/// them all, else any token.
pub fn search(
    conn: &Connection,
    tokens: &[String],
    filter: &Filter<'_>,
) -> Result<(Match, Vec<i64>)> {
    if tokens.is_empty() {
        return Ok((Match::None, Vec::new()));
    }
    for (operator, step) in [("AND", Match::All), ("OR", Match::Any)] {
        let ids = run(conn, &fts_query(tokens, operator), filter)?;
        if !ids.is_empty() {
            return Ok((step, ids));
        }
        if tokens.len() == 1 {
            break;
        }
    }
    Ok((Match::None, Vec::new()))
}

fn run(conn: &Connection, query: &str, filter: &Filter<'_>) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT e.id FROM entries_fts f JOIN entries e ON e.id = f.rowid
         WHERE entries_fts MATCH ?1
           AND e.merged_into IS NULL
           AND (?2 IS NULL OR e.status = ?2)
           AND (?3 IS NULL OR e.id != ?3)
         ORDER BY bm25(entries_fts, 4.0, 2.0, 1.0, 1.0, 2.0, 1.0), e.id DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![query, filter.status, filter.exclude, filter.limit as i64],
        |row| row.get(0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_hostile_input() {
        let tokens = tokenize(r#"error: can't find gem "foo" (>= 0) -- NEAR/3 AND OR *"#);
        assert_eq!(
            tokens,
            [
                "error", "can", "t", "find", "gem", "foo", "0", "near", "3", "and", "or"
            ]
        );
        assert_eq!(fts_query(&tokens[..2], "AND"), r#""error" AND "can""#);
        assert!(tokenize("--- !!! ***").is_empty());
    }

    #[test]
    fn hostile_query_is_valid_fts5() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(body);")
            .unwrap();
        let tokens =
            tokenize(r#"error: can't find gem "foo" (>= 0) -- NEAR/3 AND OR * ^col: {a b}"#);
        for operator in ["AND", "OR"] {
            let query = fts_query(&tokens, operator);
            conn.query_row("SELECT count(*) FROM t WHERE t MATCH ?1", [&query], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        }
    }
}
