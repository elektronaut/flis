//! Parses the Markdown log that snags replaces: a header ending in a `---`
//! line, then one entry per line as `YYYY-MM-DD · symptom · fix · project`,
//! with indented detail lines under an entry.

use std::collections::HashMap;

use serde::Serialize;
use time::Date;
use time::macros::format_description;

const SEPARATOR: &str = " · ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub line: usize,
    /// `YYYY-MM-DD`
    pub date: String,
    pub symptom: String,
    pub fix: Option<String>,
    /// The project as written.
    pub project_raw: String,
    /// The project after normalisation and the project map.
    pub project: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Malformed {
    pub line: usize,
    pub text: String,
    /// Indented lines directly under it.
    pub detail: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct ParsedLog {
    /// Whether a `---` line ended a header.
    pub header: bool,
    /// Non-blank lines after the header.
    pub lines: usize,
    pub detail_lines: usize,
    pub entries: Vec<LogEntry>,
    pub malformed: Vec<Malformed>,
}

enum Last {
    None,
    Entry,
    Malformed,
}

pub fn parse(text: &str) -> ParsedLog {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|line| line.trim_end() == "---");
    let mut log = ParsedLog {
        header: start.is_some(),
        ..ParsedLog::default()
    };
    let mut detail: Vec<&str> = Vec::new();
    let mut last = Last::None;
    let first = start.map_or(0, |index| index + 1);
    for (index, raw) in lines.iter().enumerate().skip(first) {
        let number = index + 1;
        let line = raw.trim_end();
        if line.is_empty() {
            finish_detail(&mut log, &last, &mut detail);
            last = Last::None;
            continue;
        }
        log.lines += 1;
        if line.starts_with([' ', '\t']) && !matches!(last, Last::None) {
            log.detail_lines += 1;
            detail.push(line);
            continue;
        }
        finish_detail(&mut log, &last, &mut detail);
        match parse_line(line, number) {
            Some(entry) => {
                log.entries.push(entry);
                last = Last::Entry;
            }
            None => {
                log.malformed.push(Malformed {
                    line: number,
                    text: line.to_owned(),
                    detail: Vec::new(),
                });
                last = Last::Malformed;
            }
        }
    }
    finish_detail(&mut log, &last, &mut detail);
    log
}

fn finish_detail(log: &mut ParsedLog, last: &Last, detail: &mut Vec<&str>) {
    if detail.is_empty() {
        return;
    }
    let indent = detail
        .iter()
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let lines: Vec<String> = detail
        .iter()
        .map(|line| line[indent..].to_owned())
        .collect();
    match last {
        Last::Entry => {
            if let Some(entry) = log.entries.last_mut() {
                entry.detail = Some(lines.join("\n"));
            }
        }
        Last::Malformed => {
            if let Some(malformed) = log.malformed.last_mut() {
                malformed.detail = lines;
            }
        }
        Last::None => {}
    }
    detail.clear();
}

fn parse_line(line: &str, number: usize) -> Option<LogEntry> {
    let body = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    let fields: Vec<&str> = body.split(SEPARATOR).map(str::trim).collect();
    let [date, symptom, fix, project] = fields.as_slice() else {
        return None;
    };
    Date::parse(date, format_description!("[year]-[month]-[day]")).ok()?;
    if symptom.is_empty() {
        return None;
    }
    Some(LogEntry {
        line: number,
        date: (*date).to_owned(),
        symptom: (*symptom).to_owned(),
        fix: (!fix.is_empty() && *fix != "-").then(|| (*fix).to_owned()),
        project_raw: (*project).to_owned(),
        project: normalize_project(project),
        detail: None,
    })
}

/// Trims, drops a trailing parenthetical and a trailing ` #123`, and turns
/// `-`, `global` and nothing into no project.
pub fn normalize_project(raw: &str) -> Option<String> {
    let mut name = raw.trim();
    loop {
        let before = name;
        if let Some(position) = name.rfind(" #")
            && name[position + 2..].chars().all(|c| c.is_ascii_digit())
            && name.len() > position + 2
        {
            name = name[..position].trim_end();
        }
        if name.ends_with(')')
            && let Some(open) = matching_open(name)
        {
            name = name[..open].trim_end();
        }
        if name == before {
            break;
        }
    }
    (!name.is_empty() && name != "-" && !name.eq_ignore_ascii_case("global"))
        .then(|| name.to_owned())
}

/// The position of the `(` that the final `)` closes.
fn matching_open(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (index, c) in text.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Applies a person's mapping of old project names (after normalisation)
/// to new ones or to no project. Returns the keys that matched nothing.
pub fn apply_project_map(
    entries: &mut [LogEntry],
    map: &HashMap<String, Option<String>>,
) -> Vec<String> {
    let mut used = Vec::new();
    for entry in entries.iter_mut() {
        if let Some(name) = &entry.project
            && let Some(target) = map.get(name)
        {
            if !used.contains(name) {
                used.push(name.clone());
            }
            entry.project = target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(str::to_owned);
        }
    }
    let mut unused: Vec<String> = map
        .keys()
        .filter(|key| !used.contains(key))
        .cloned()
        .collect();
    unused.sort();
    unused
}

/// FNV-1a, 64 bits, as hex.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Papercuts

Format: date · symptom · fix · project

---

2026-09-01 · bundle install fails · run bin/setup · acme
  saw it twice
  - nested detail
- 2026-09-02 · gh pr create hangs · - · nova (ticket 1024)

2026-09-03 · only three · fields
  detail of a malformed line
2026-09-04 · a · b · c · d
not an entry at all
2026-13-40 · bad date · x · shop
2026-09-05 ·  · empty symptom · shop
* 2026-09-06 · npm ci slow · use the cache · shop #12
 \t
   orphan after a blank line
";

    #[test]
    fn parses_entries_detail_and_malformed_lines() {
        let log = parse(LOG);
        assert!(log.header);
        assert_eq!(log.entries.len(), 3);
        let first = &log.entries[0];
        assert_eq!(first.line, 7);
        assert_eq!(first.date, "2026-09-01");
        assert_eq!(first.symptom, "bundle install fails");
        assert_eq!(first.fix.as_deref(), Some("run bin/setup"));
        assert_eq!(first.project.as_deref(), Some("acme"));
        assert_eq!(
            first.detail.as_deref(),
            Some("saw it twice\n- nested detail")
        );

        let second = &log.entries[1];
        assert_eq!(second.fix, None);
        assert_eq!(second.project_raw, "nova (ticket 1024)");
        assert_eq!(second.project.as_deref(), Some("nova"));
        assert_eq!(second.detail, None);

        assert_eq!(log.entries[2].project.as_deref(), Some("shop"));
        assert_eq!(log.entries[2].line, 18);

        let malformed: Vec<usize> = log.malformed.iter().map(|m| m.line).collect();
        assert_eq!(malformed, [12, 14, 15, 16, 17, 20]);
        assert_eq!(log.malformed[0].text, "2026-09-03 · only three · fields");
        assert_eq!(log.malformed[0].detail, ["detail of a malformed line"]);
        assert_eq!(log.malformed[5].text, "   orphan after a blank line");
        assert_eq!(log.detail_lines, 3);
    }

    #[test]
    fn without_a_header_the_whole_file_is_read() {
        let log = parse("2026-09-01 · a · b · acme\n");
        assert!(!log.header);
        assert_eq!(log.entries.len(), 1);
        assert_eq!(parse("").entries.len(), 0);
    }

    #[test]
    fn normalises_projects_generically() {
        let cases = [
            ("acme", Some("acme")),
            ("  acme  ", Some("acme")),
            ("-", None),
            ("", None),
            ("global", None),
            ("Global", None),
            ("shop (ticket 1024)", Some("shop")),
            ("shop #123", Some("shop")),
            ("shop (beta) #12", Some("shop")),
            ("shop #12 (beta)", Some("shop")),
            ("nova (a (b) c)", Some("nova")),
            ("(nothing)", None),
            ("acme/mono", Some("acme/mono")),
            ("mono #x", Some("mono #x")),
            ("mono#12", Some("mono#12")),
            ("global (all)", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_project(raw).as_deref(), expected, "{raw:?}");
        }
    }

    #[test]
    fn maps_projects_and_reports_unused_keys() {
        let mut log = parse(
            "---\n2026-09-01 · a · b · acme\n2026-09-02 · c · d · nova\n2026-09-03 · e · f · shop\n",
        );
        let map = HashMap::from([
            ("acme".to_owned(), Some("acme/apps".to_owned())),
            ("nova".to_owned(), None),
            ("mono".to_owned(), Some("x".to_owned())),
        ]);
        let unused = apply_project_map(&mut log.entries, &map);
        let projects: Vec<Option<&str>> =
            log.entries.iter().map(|e| e.project.as_deref()).collect();
        assert_eq!(projects, [Some("acme/apps"), None, Some("shop")]);
        assert_eq!(unused, ["mono"]);
    }

    #[test]
    fn hashes_content() {
        assert_eq!(content_hash(b""), "cbf29ce484222325");
        assert_eq!(content_hash(b"a"), "af63dc4c8601ec8c");
        assert_ne!(content_hash(b"ab"), content_hash(b"ba"));
    }
}
