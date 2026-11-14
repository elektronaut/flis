use std::fmt::Write;

use crate::clock::date_part;
use crate::model::Entry;
use crate::store::Seen;

pub trait Report: serde::Serialize {
    fn text(&self) -> String;
}

/// The text on one line, cut at `max` characters.
pub fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// `label: text`, with later lines of the text indented under the first.
pub fn field(out: &mut String, indent: &str, label: &str, text: &str) {
    let mut lines = text.trim_end().lines();
    let _ = writeln!(out, "{indent}{label}: {}", lines.next().unwrap_or(""));
    let pad = " ".repeat(label.len() + 2);
    for line in lines {
        let _ = writeln!(out, "{indent}{pad}{line}");
    }
}

pub fn plural(count: impl Into<u64>, one: &str, many: &str) -> String {
    let count = count.into();
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn seen_text(seen: &Seen) -> String {
    match &seen.last_seen {
        Some(last) => format!(
            "seen {}, last {}",
            plural(seen.count.max(0) as u64, "time", "times"),
            date_part(last)
        ),
        None => "never seen".to_owned(),
    }
}

/// `4 open · bundler · silent`
pub fn entry_heading(entry: &Entry) -> String {
    let mut heading = format!("{} {}", entry.id, entry.status);
    if let Some(subject) = &entry.subject {
        let _ = write!(heading, " · {subject}");
    }
    if entry.silent {
        heading.push_str(" · SILENT");
    }
    heading
}

pub fn resolution_text(entry: &Entry) -> Option<String> {
    if entry.fix_location.is_none() && entry.reference.is_none() && entry.resolution_note.is_none()
    {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(location) = entry.fix_location {
        parts.push(format!("in {location}"));
    }
    if let Some(reference) = &entry.reference {
        parts.push(format!("ref {reference}"));
    }
    if let Some(note) = &entry.resolution_note {
        parts.push(note.clone());
    }
    Some(parts.join(" · "))
}
