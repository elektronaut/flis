use serde_json::Value;

use crate::common::Snags;

fn ids(report: &Value) -> Vec<i64> {
    report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect()
}

const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

fn seeded() -> Snags {
    let mut snags = Snags::new();
    snags.at("2026-06-01T10:00:00Z");
    snags.add(&["old and forgotten", "--cost", "blocker"]);
    snags.at("2026-11-01T10:00:00Z");
    let busy = snags.add(&["busy failure", "--project", "acme", "--cost", "major"]);
    snags.at("2026-11-02T10:00:00Z");
    snags
        .run(&["seen", &busy.to_string(), "--project", "nova"])
        .ok();
    snags.add(&["quiet failure"]);
    snags.add(&["silent failure", "--silent"]);
    let waiting = snags.add(&["upstream failure", "--cost", "blocker"]);
    snags
        .run(&["edit", &waiting.to_string(), "--ref", "acme/tool#9"])
        .ok();
    let done = snags.add(&["fixed failure"]);
    snags
        .run(&[
            "resolve",
            &done.to_string(),
            "--where",
            "setup",
            "--ref",
            "abc",
        ])
        .ok();
    let dup = snags.add(&["busy failure again"]);
    snags
        .run(&["merge", &dup.to_string(), "--into", &busy.to_string()])
        .ok();
    snags.at("2026-11-14T12:00:00Z");
    snags
}

#[test]
fn exports_complete_entries_in_rank_order() {
    let snags = seeded();
    let report = snags.json(&["export"]);
    assert_eq!(ids(&report), [4, 2, 3, 1, 5]);
    assert_eq!(report["total"], 5);
    assert_eq!(report["omitted"], 0);
    assert_eq!(report["status"], "open");
    assert_eq!(
        report["entries"][1]["occurrences"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(report["entries"][1]["rank"]["score"], 30);
    assert_eq!(report["entries"][3]["rank"]["occurrences"], 0);
    assert_eq!(report["entries"][4]["waiting"], true);

    let all = snags.json(&["export", "--status", "all"]);
    assert_eq!(all["total"], 6);
    let resolved = snags.json(&["export", "--status", "resolved"]);
    assert_eq!(ids(&resolved), [6]);

    let text = snags.run(&["export"]).ok().stdout;
    assert!(
        text.starts_with("# snags export\n\n5 open entries,"),
        "{text}"
    );
    assert!(text.contains("\n## 4 open · SILENT\n"), "{text}");
    assert!(text.contains("- symptom: busy failure\n"), "{text}");
    assert!(
        text.contains("  Merged from 7: busy failure again\n"),
        "{text}"
    );
    assert!(
        text.contains("- o3 · 2026-11-02T10:00:00Z · minor · nova · claude-code"),
        "{text}"
    );
    assert!(text.contains("- reported: waiting on the ref"), "{text}");
    assert!(text.contains("Not seen in the window."), "{text}");
    let order: Vec<usize> = ["## 4 ", "## 2 ", "## 3 ", "## 1 ", "## 5 "]
        .iter()
        .map(|heading| text.find(heading).unwrap())
        .collect();
    assert!(order.is_sorted(), "{text}");

    let json_format = snags.run(&["export", "--format", "json"]).ok();
    let mut from_format = json_format.json();
    let mut from_flag = snags.json(&["export"]);
    from_format["estimated_tokens"] = Value::Null;
    from_flag["estimated_tokens"] = Value::Null;
    assert_eq!(from_format, from_flag);
}

#[test]
fn stops_at_the_token_budget() {
    let snags = seeded();
    for format in ["md", "json"] {
        let full = snags.run(&["export", "--format", format]).ok().stdout;
        for budget in [150, 300, 500] {
            let out = snags
                .run(&[
                    "export",
                    "--format",
                    format,
                    "--max-tokens",
                    &budget.to_string(),
                ])
                .ok()
                .stdout;
            assert!(
                out.len().div_ceil(4) <= budget,
                "{format} {budget}: {} chars\n{out}",
                out.len()
            );
            assert!(out.len() <= full.len(), "{format} {budget}");
        }
    }

    let report = snags.json(&["export", "--max-tokens", "500"]);
    let kept = ids(&report);
    assert!(!kept.is_empty() && kept.len() < 5, "{kept:?}");
    assert_eq!(kept, [4, 2, 3, 1, 5][..kept.len()]);
    assert_eq!(report["omitted"], 5 - kept.len());
    assert_eq!(report["max_tokens"], 500);
    assert!(report["estimated_tokens"].as_u64().unwrap() <= 500);

    let text = snags.run(&["export", "--max-tokens", "300"]).ok().stdout;
    assert!(
        text.contains("left out to stay within --max-tokens 300."),
        "{text}"
    );
    assert!(
        text.contains("more entries left out; raise --max-tokens"),
        "{text}"
    );

    let tiny = snags.run(&["export", "--max-tokens", "60"]).ok().stdout;
    assert!(tiny.contains("0 of 5 open entries"), "{tiny}");
    assert!(tiny.contains("The first entry alone needs about"), "{tiny}");

    let big = snags.json(&["export", "--max-tokens", "100000"]);
    assert_eq!(big["omitted"], 0);
    assert_eq!(snags.run(&["export", "--max-tokens", "0"]).code, 2);
    assert_eq!(snags.run(&["export", "--format", "yaml"]).code, 2);
}

#[test]
fn masks_secrets_on_the_way_out_only() {
    let snags = Snags::new();
    let symptom =
        format!("git push asked for a password; remote is https://{TOKEN}@github.com/acme/shop");
    let id = snags.add(&[
        &symptom,
        "--workaround",
        "export DB_PASSWORD=hunter2 and curl -H 'Authorization: Bearer abc.def'",
        "--note=-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----",
    ]);
    snags.add(&[
        "cargo test failed in src/models/secret_store.rs:42 at commit 3f2a9c1e5b7d4f6a8c0e2b4d6f8a0c2e4b6d8f0a",
    ]);

    let text = snags.run(&["export"]).ok().stdout;
    assert!(!text.contains(TOKEN), "{text}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("abc.def"), "{text}");
    assert!(!text.contains("b3BlbnNzaC1rZXktdjEAAAAA"), "{text}");
    assert!(
        text.contains("https://[masked]@github.com/acme/shop"),
        "{text}"
    );
    assert!(
        text.contains("export DB_PASSWORD=[masked] and curl -H 'Authorization: Bearer [masked]'"),
        "{text}"
    );
    assert!(text.contains("  [masked private key]\n"), "{text}");
    assert!(text.contains("4 secrets masked in this output"), "{text}");
    assert!(
        text.contains(
            "src/models/secret_store.rs:42 at commit 3f2a9c1e5b7d4f6a8c0e2b4d6f8a0c2e4b6d8f0a"
        ),
        "{text}"
    );

    let report = snags.json(&["export"]);
    assert_eq!(report["masked"], 4);
    assert!(!report.to_string().contains(TOKEN));

    let show = snags.json(&["show", &id.to_string()]);
    assert_eq!(show["entry"]["symptom"], symptom.as_str());
    assert!(
        show["occurrences"][0]["note"]
            .as_str()
            .unwrap()
            .contains("b3BlbnNzaC1rZXktdjEAAAAA")
    );
    assert_eq!(snags.json(&["search", "hunter2"])["hits"][0]["id"], id);
}

#[test]
fn export_of_an_empty_database() {
    let snags = Snags::new();
    let report = snags.json(&["export", "--max-tokens", "10"]);
    assert_eq!(report["total"], 0);
    assert_eq!(report["entries"], serde_json::json!([]));
    let text = snags.run(&["export"]).ok().stdout;
    assert!(
        text.starts_with("# snags export\n\n0 open entries"),
        "{text}"
    );
}
