use serde_json::json;

use crate::common::Flis;

#[test]
fn stats_counts_by_status_project_and_subject() {
    let mut flis = Flis::new();
    flis.at("2026-10-01T10:00:00Z");
    flis.add(&["old one", "--subject", "git", "--project", "acme"]);
    flis.at("2026-11-10T10:00:00Z");
    let a = flis.add(&["gh token expired", "--subject", "gh", "--project", "acme"]);
    flis.run(&["seen", &a.to_string(), "--project", "nova"])
        .ok();
    let b = flis.add(&[
        "gh rate limited",
        "--subject",
        "gh",
        "--silent",
        "--project",
        "nova",
    ]);
    let c = flis.add(&["git hook slow", "--subject", "git"]);
    let d = flis.add(&["no subject here", "--project", "shop"]);
    let dup = flis.add(&["dup of d", "--subject", "dup", "--project", "shop"]);
    flis.at("2026-11-12T10:00:00Z");
    flis.run(&["edit", &b.to_string(), "--ref", "acme/gh#1"])
        .ok();
    flis.run(&["resolve", &c.to_string(), "--where", "setup", "--ref", "x"])
        .ok();
    flis.run(&["seen", &c.to_string()]).ok();
    flis.run(&["resolve", &c.to_string(), "--where", "setup", "--ref", "y"])
        .ok();
    flis.run(&["accept", &d.to_string(), "--note", "known"])
        .ok();
    flis.run(&["merge", &dup.to_string(), "--into", &d.to_string()])
        .ok();

    flis.at("2026-11-14T12:00:00Z");
    let stats = flis.json(&["stats"]);
    assert!(stats["since"].is_null());
    assert_eq!(stats["entries"], 5);
    assert_eq!(stats["occurrences"], 8);
    assert_eq!(stats["first_seen"], "2026-10-01T10:00:00Z");
    assert_eq!(stats["last_seen"], "2026-11-12T10:00:00Z");
    assert_eq!(
        stats["status"],
        json!({"open": 3, "resolved": 1, "accepted": 1})
    );
    assert_eq!(stats["waiting"], 1);
    assert_eq!(stats["silent"], 1);
    assert_eq!(stats["regressions"], 1);
    assert_eq!(
        stats["projects"],
        json!([
            {"name": "acme", "entries": 2, "occurrences": 2},
            {"name": "nova", "entries": 2, "occurrences": 2},
            {"name": "shop", "entries": 1, "occurrences": 2},
            {"name": null, "entries": 1, "occurrences": 2},
        ])
    );
    assert_eq!(
        stats["subjects"],
        json!([
            {"name": "gh", "entries": 2, "occurrences": 3},
            {"name": "git", "entries": 2, "occurrences": 3},
            {"name": null, "entries": 1, "occurrences": 2},
        ])
    );

    let recent = flis.json(&["stats", "--since", "2026-11-01"]);
    assert_eq!(recent["since"], "2026-11-01T00:00:00Z");
    assert_eq!(recent["entries"], 4);
    assert_eq!(recent["occurrences"], 7);
    assert_eq!(
        recent["subjects"][1],
        json!({"name": "git", "entries": 1, "occurrences": 2})
    );
    assert_eq!(
        recent["projects"][3],
        json!({"name": "acme", "entries": 1, "occurrences": 1})
    );

    let text = flis.run(&["stats"]).ok().stdout;
    assert!(
        text.starts_with("All entries.\n5 entries, 8 occurrences, 2026-10-01 to 2026-11-12.\n"),
        "{text}"
    );
    assert!(
        text.contains("Status: 3 open (1 reported, waiting), 1 resolved, 1 accepted."),
        "{text}"
    );
    assert!(
        text.contains("  gh            2 entries · 3 occurrences"),
        "{text}"
    );
    assert!(
        text.contains("  (no project)  1 entry · 2 occurrences"),
        "{text}"
    );
    assert!(!text.contains("dup"), "{text}");

    assert_eq!(flis.run(&["stats", "--since", "soon"]).code, 2);
}

#[test]
fn stats_on_an_empty_database() {
    let flis = Flis::new();
    let stats = flis.json(&["stats"]);
    assert_eq!(stats["entries"], 0);
    assert_eq!(stats["projects"], json!([]));
    let text = flis.run(&["stats"]).ok().stdout;
    assert!(text.contains("0 entries, 0 occurrences."), "{text}");
}
