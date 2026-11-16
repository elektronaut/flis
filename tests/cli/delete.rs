use serde_json::json;

use crate::common::Snags;

fn count(snags: &Snags, sql: &str) -> i64 {
    snags.sql().query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Rows in each table: entries, occurrences, entry_history, entries_fts.
fn counts(snags: &Snags) -> [i64; 4] {
    [
        count(snags, "SELECT count(*) FROM entries"),
        count(snags, "SELECT count(*) FROM occurrences"),
        count(snags, "SELECT count(*) FROM entry_history"),
        count(snags, "SELECT count(*) FROM entries_fts"),
    ]
}

fn search_ids(snags: &Snags, text: &str) -> Vec<i64> {
    snags.json(&["search", text])["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn delete_removes_an_entry_with_its_occurrences_history_and_search_row() {
    let snags = Snags::new();
    let gone = snags.add(&["shop deploy hangs on asset precompile", "--project", "shop"]);
    let kept = snags.add(&["nova tests flake on timezone", "--project", "nova"]);
    snags
        .run(&["seen", &gone.to_string(), "--note", "again"])
        .ok();
    snags
        .run(&[
            "edit",
            &gone.to_string(),
            "--workaround",
            "precompile locally",
        ])
        .ok();
    assert_eq!(search_ids(&snags, "precompile"), vec![gone]);

    let report = snags.json(&["delete", &gone.to_string()]);
    assert_eq!(report["dry_run"], false);
    assert_eq!(
        report["entries"],
        json!([{"id": gone, "occurrences": [1, 3]}])
    );
    assert_eq!(report["occurrences"], json!([1, 3]));
    assert_eq!(report["kept"], json!([]));

    assert_eq!(counts(&snags), [1, 1, 0, 1]);
    assert_eq!(snags.run(&["show", &gone.to_string()]).code, 3);
    assert!(search_ids(&snags, "precompile").is_empty());
    assert_eq!(search_ids(&snags, "timezone"), vec![kept]);
}

#[test]
fn delete_follows_a_merged_id_and_removes_the_stubs() {
    let snags = Snags::new();
    let a = snags.add(&["alpha failure"]);
    let b = snags.add(&["beta failure"]);
    let c = snags.add(&["gamma failure"]);
    let other = snags.add(&["delta failure"]);
    snags
        .run(&["merge", &a.to_string(), "--into", &b.to_string()])
        .ok();
    snags
        .run(&["merge", &b.to_string(), "--into", &c.to_string()])
        .ok();
    snags
        .run(&[
            "resolve",
            &c.to_string(),
            "--where",
            "setup",
            "--ref",
            "abc123",
        ])
        .ok();

    let report = snags.json(&["delete", &a.to_string()]);
    assert_eq!(report["merged_from"], json!([{"given": a, "id": c}]));
    assert_eq!(report["entries"][0]["id"], c);
    assert_eq!(report["entries"][0]["occurrences"], json!([1, 2, 3]));
    assert_eq!(
        report["stubs"],
        json!([{"id": a, "merged_into": b}, {"id": b, "merged_into": c}])
    );

    let ids: Vec<i64> = snags
        .sql()
        .prepare("SELECT id FROM entries ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, vec![other]);
    assert_eq!(counts(&snags), [1, 1, 0, 1]);
}

#[test]
fn delete_occurrences_keeps_the_entry_and_reindexes_it() {
    let mut snags = Snags::new();
    let id = snags
        .add(&["mono build slow", "--note", "first time"])
        .to_string();
    snags.at("2026-11-15T09:00:00Z");
    snags
        .run(&["seen", &id, "--note", "cache evicted by cleanup"])
        .ok();
    snags.run(&["seen", &id, "--note", "third"]).ok();
    assert_eq!(search_ids(&snags, "evicted").len(), 1);

    let report = snags.json(&["delete", "--occurrence", "o2", "3"]);
    assert_eq!(report["entries"], json!([]));
    assert_eq!(
        report["kept"],
        json!([{"id": 1, "deleted": [2, 3], "remaining": 1}])
    );
    assert_eq!(report["occurrences"], json!([2, 3]));

    let show = snags.json(&["show", &id]);
    assert_eq!(show["occurrences"].as_array().unwrap().len(), 1);
    assert_eq!(show["occurrences"][0]["id"], 1);
    assert_eq!(show["entry"]["updated_at"], "2026-11-15T09:00:00Z");
    assert!(search_ids(&snags, "evicted").is_empty());

    let text = snags
        .run(&["delete", "--occurrence", "o1", "--dry-run"])
        .ok()
        .stdout;
    assert!(
        text.contains("Would delete entry 1 with 1 occurrence: o1."),
        "{text}"
    );
}

#[test]
fn deleting_the_last_occurrence_deletes_the_entry() {
    let snags = Snags::new();
    let id = snags.add(&["acme migrations lock the table"]);
    snags.add(&["unrelated"]);

    let text = snags.run(&["delete", "--occurrence", "o1"]).ok().stdout;
    assert!(
        text.contains(&format!("Deleted entry {id} with 1 occurrence: o1.")),
        "{text}"
    );
    assert!(text.contains("There is no undo."), "{text}");
    assert_eq!(snags.run(&["show", &id.to_string()]).code, 3);
    assert_eq!(counts(&snags), [1, 1, 0, 1]);
}

#[test]
fn delete_project_takes_its_occurrences_out_of_mixed_entries() {
    let snags = Snags::new();
    let mixed = snags.add(&["gh auth token expired", "--project", "acme"]);
    snags
        .run(&["seen", &mixed.to_string(), "--project", "nova"])
        .ok();
    snags
        .run(&["seen", &mixed.to_string(), "--project", "acme"])
        .ok();
    let only_acme = snags.add(&["acme seeds fail", "--project", "acme"]);
    let only_shop = snags.add(&["shop assets missing", "--project", "shop"]);
    let near_name = snags.add(&["acme2 lint fails", "--project", "acme2"]);
    snags
        .run(&["accept", &only_acme.to_string(), "--note", "legacy"])
        .ok();

    let report = snags.json(&["delete", "--project", "acme", "shop"]);
    let entries: Vec<i64> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_i64().unwrap())
        .collect();
    assert_eq!(entries, vec![only_acme, only_shop]);
    assert_eq!(
        report["kept"],
        json!([{"id": mixed, "deleted": [1, 3], "remaining": 1}])
    );

    let show = snags.json(&["show", &mixed.to_string()]);
    assert_eq!(show["occurrences"][0]["project"], "nova");
    assert_eq!(show["occurrences"].as_array().unwrap().len(), 1);
    assert_eq!(
        snags.json(&["show", &near_name.to_string()])["entry"]["id"],
        near_name
    );
    assert_eq!(count(&snags, "SELECT count(*) FROM entries"), 2);

    let report = snags.json(&["delete", "--project", "acme"]);
    assert_eq!(report["occurrences"], json!([]));
    let text = snags.run(&["delete", "--project", "acme"]).ok().stdout;
    assert_eq!(text, "Nothing to delete.\n");
}

#[test]
fn dry_run_reports_and_writes_nothing() {
    let snags = Snags::new();
    let a = snags.add(&["alpha failure", "--project", "acme"]);
    let b = snags.add(&["beta failure", "--project", "acme"]);
    snags
        .run(&["seen", &b.to_string(), "--project", "nova"])
        .ok();
    snags
        .run(&["edit", &a.to_string(), "--workaround", "retry"])
        .ok();
    let before = counts(&snags);

    let report = snags.json(&["delete", &a.to_string(), "--project", "acme", "--dry-run"]);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["entries"][0]["id"], a);
    assert_eq!(report["kept"][0]["id"], b);

    let text = snags
        .run(&["delete", &a.to_string(), "--project", "acme", "--dry-run"])
        .ok()
        .stdout;
    assert!(text.contains(&format!("Would delete entry {a}")), "{text}");
    assert!(text.contains("which would keep 1 occurrence"), "{text}");
    assert!(text.contains("nothing was deleted"), "{text}");

    assert_eq!(counts(&snags), before);
    assert_eq!(search_ids(&snags, "alpha"), vec![a]);
}

#[test]
fn delete_needs_a_selector_and_refuses_unknown_ids_without_writing() {
    let snags = Snags::new();
    let a = snags.add(&["alpha failure"]).to_string();
    let before = counts(&snags);

    let run = snags.run(&["delete"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("--occurrence or --project"),
        "{}",
        run.stderr
    );
    assert_eq!(snags.run(&["delete", "--json"]).json()["error"]["code"], 2);

    let run = snags.run(&["delete", &a, "99"]);
    assert_eq!(run.code, 3);
    assert!(run.stderr.contains("no entry 99"), "{}", run.stderr);

    let run = snags.run(&["delete", &a, "--occurrence", "o42"]);
    assert_eq!(run.code, 3);
    assert!(run.stderr.contains("no occurrence o42"), "{}", run.stderr);

    assert_eq!(snags.run(&["delete", "--occurrence", "x1"]).code, 2);
    assert_eq!(counts(&snags), before);
}
