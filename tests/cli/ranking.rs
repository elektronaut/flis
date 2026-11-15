use serde_json::Value;

use crate::common::Snags;

fn ids(group: &Value) -> Vec<i64> {
    group["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn ranks_silent_first_then_by_score_then_recency() {
    let mut snags = Snags::new();
    snags.at("2026-11-01T10:00:00Z");
    let busy = snags.add(&["busy", "--project", "acme", "--cost", "major"]);
    let quiet_old = snags.add(&["quiet old"]);
    let silent = snags.add(&["silent one", "--silent"]);
    snags.at("2026-11-02T10:00:00Z");
    snags
        .run(&["seen", &busy.to_string(), "--project", "nova"])
        .ok();
    snags.at("2026-11-03T10:00:00Z");
    let quiet_new = snags.add(&["quiet new"]);

    snags.at("2026-11-14T12:00:00Z");
    let list = snags.json(&["list"]);
    assert_eq!(ids(&list["ranked"]), [silent, busy, quiet_new, quiet_old]);

    let busy_row = &list["ranked"]["entries"][1];
    assert_eq!(busy_row["weight"], 4);
    assert_eq!(busy_row["days"], 2);
    assert_eq!(busy_row["projects"], 2);
    assert_eq!(busy_row["score"], 16);
    assert_eq!(busy_row["last_seen"], "2026-11-02T10:00:00Z");

    let text = snags.run(&["list"]).ok().stdout;
    assert!(
        text.contains(&format!(
            "{busy} open · score 16 = weight 4 × 2 days × 2 projects · 2 occurrences, last 2026-11-02"
        )),
        "{text}"
    );

    let recent = snags.json(&["list", "--sort", "recent"]);
    assert_eq!(ids(&recent["ranked"]), [quiet_new, busy, silent, quiet_old]);
}

#[test]
fn only_occurrences_inside_the_window_count() {
    let mut snags = Snags::new();
    snags.at("2026-06-01T10:00:00Z");
    let old = snags.add(&["old and frequent", "--cost", "blocker"]);
    for day in ["02", "03", "04"] {
        snags.at(&format!("2026-06-{day}T10:00:00Z"));
        snags
            .run(&["seen", &old.to_string(), "--cost", "blocker"])
            .ok();
    }
    snags.at("2026-11-10T10:00:00Z");
    let fresh = snags.add(&["fresh", "--cost", "major"]);
    let dormant = snags.add(&["dormant"]);
    snags
        .sql()
        .execute(
            "UPDATE occurrences SET at = '2026-06-05T10:00:00Z' WHERE entry_id = ?1",
            [dormant],
        )
        .unwrap();
    snags.at("2026-11-12T10:00:00Z");
    snags.run(&["seen", &old.to_string()]).ok();

    snags.at("2026-11-14T12:00:00Z");
    let list = snags.json(&["list"]);
    assert_eq!(ids(&list["ranked"]), [fresh, old]);
    assert_eq!(list["ranked"]["entries"][1]["weight"], 1);
    assert_eq!(list["not_seen_since"], 1);
    assert_eq!(list["since"], "2026-08-16T12:00:00Z");

    let wide = snags.json(&["list", "--since", "2026-01-01"]);
    assert_eq!(ids(&wide["ranked"]), [old, fresh, dormant]);
    assert_eq!(wide["ranked"]["entries"][0]["score"], 21 * 5);

    let text = snags.run(&["list"]).ok().stdout;
    assert!(
        text.contains("1 more entry not seen since 2026-08-16"),
        "{text}"
    );
}

#[test]
fn reported_entries_wait_in_their_own_group() {
    let snags = Snags::new();
    let plain = snags.add(&["plain"]);
    let reported = snags.add(&["reported upstream", "--cost", "blocker"]);
    snags
        .run(&[
            "edit",
            &reported.to_string(),
            "--ref",
            "https://example.com/issues/1",
        ])
        .ok();

    let list = snags.json(&["list"]);
    assert_eq!(ids(&list["ranked"]), [plain]);
    assert_eq!(ids(&list["waiting"]), [reported]);

    let text = snags.run(&["list"]).ok().stdout;
    let waiting_at = text.find("Reported, waiting").unwrap();
    assert!(text.find("plain").unwrap() < waiting_at, "{text}");
    assert!(
        text[waiting_at..].contains("ref: https://example.com/issues/1"),
        "{text}"
    );
}

#[test]
fn filters_and_limit() {
    let snags = Snags::new();
    let acme = snags.add(&["one", "--project", "acme", "--subject", "git"]);
    let nova = snags.add(&["two", "--project", "nova", "--subject", "gh"]);
    let done = snags.add(&["three", "--project", "acme", "--subject", "git"]);
    snags
        .run(&[
            "resolve",
            &done.to_string(),
            "--where",
            "setup",
            "--ref",
            "x",
        ])
        .ok();

    assert_eq!(
        ids(&snags.json(&["list", "--project", "acme"])["ranked"]),
        [acme]
    );
    assert_eq!(
        ids(&snags.json(&["list", "--subject", "GH"])["ranked"]),
        [nova]
    );
    assert_eq!(
        ids(&snags.json(&["list", "--status", "resolved"])["ranked"]),
        [done]
    );
    assert_eq!(
        ids(&snags.json(&["list", "--status", "all"])["ranked"]).len(),
        3
    );

    let limited = snags.json(&["list", "--limit", "1"]);
    assert_eq!(ids(&limited["ranked"]).len(), 1);
    assert_eq!(limited["ranked"]["more"], 1);

    let empty = snags.run(&["list", "--subject", "docker"]).ok().stdout;
    assert!(empty.contains("None."), "{empty}");
}
