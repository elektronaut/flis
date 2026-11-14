use crate::common::Flis;

#[test]
fn accept_requires_a_reason_and_keeps_it() {
    let mut flis = Flis::new();
    let id = flis.add(&["macOS asks for keychain access"]).to_string();
    assert_eq!(flis.run(&["accept", &id]).code, 2);
    assert_eq!(flis.run(&["accept", &id, "--note", "  "]).code, 2);
    assert_eq!(flis.run(&["accept", "99", "--note", "x"]).code, 3);

    flis.at("2026-11-14T13:00:00Z");
    let report = flis.json(&["accept", &id, "--note", "by design, no fix planned"]);
    assert_eq!(report["previous_status"], "open");
    let entry = &report["entry"];
    assert_eq!(entry["status"], "accepted");
    assert_eq!(entry["fix_location"], "none");
    assert_eq!(entry["resolution_note"], "by design, no fix planned");
    assert_eq!(entry["updated_at"], "2026-11-14T13:00:00Z");

    let run = flis.run(&["accept", &id, "--note", "again"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("flis reopen"), "{}", run.stderr);

    let seen = flis.json(&["seen", &id]);
    assert_eq!(seen["entry"]["status"], "accepted");
    let hits = flis.run(&["search", "keychain"]).ok().stdout;
    assert!(hits.contains("accepted: by design"), "{hits}");
    let list = flis.json(&["list"]);
    assert!(list["ranked"]["entries"].as_array().unwrap().is_empty());

    flis.run(&["reopen", &id]).ok();
    let show = flis.json(&["show", &id]);
    assert_eq!(show["entry"]["status"], "open");
    assert!(show["entry"]["resolution_note"].is_null());
}

#[test]
fn accept_prints_the_reason() {
    let flis = Flis::new();
    let id = flis.add(&["slow first build"]).to_string();
    let text = flis
        .run(&["accept", &id, "--note", "cold cache"])
        .ok()
        .stdout;
    assert_eq!(text, format!("Accepted {id}: cold cache\n"));

    let resolved = flis.add(&["fixed thing"]).to_string();
    flis.run(&["resolve", &resolved, "--where", "setup", "--ref", "abc"])
        .ok();
    let run = flis.run(&["accept", &resolved, "--note", "x", "--json"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.json()["error"]["kind"], "usage");
}

fn occurrence_count(flis: &Flis, id: i64) -> usize {
    flis.json(&["show", &id.to_string()])["occurrences"]
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn merge_moves_occurrences_and_keeps_the_source_text() {
    let mut flis = Flis::new();
    flis.at("2026-11-10T10:00:00Z");
    let target = flis.add(&[
        "worktree guard refused compound command",
        "--project",
        "acme",
    ]);
    flis.at("2026-11-11T10:00:00Z");
    let source = flis.add(&[
        "guard says too complex to verify",
        "--workaround",
        "run the commands one by one",
        "--hypothesis",
        "the guard cannot parse &&",
        "--subject",
        "guard",
        "--note",
        "while running tests",
        "--project",
        "nova",
    ]);
    flis.at("2026-11-12T10:00:00Z");
    flis.run(&["seen", &source.to_string(), "--project", "shop"])
        .ok();
    let other = flis.add(&["guard blocked cd && git", "--project", "mono"]);

    flis.at("2026-11-14T12:00:00Z");
    let report = flis.json(&[
        "merge",
        &source.to_string(),
        &other.to_string(),
        "--into",
        &target.to_string(),
    ]);
    assert_eq!(report["entry"]["id"], target);
    assert_eq!(report["merged"][0]["id"], source);
    assert_eq!(report["merged"][0]["occurrences"], 2);
    assert_eq!(report["merged"][1]["occurrences"], 1);
    assert_eq!(report["seen"]["count"], 4);
    assert_eq!(report["entry"]["updated_at"], "2026-11-14T12:00:00Z");

    let show = flis.json(&["show", &target.to_string()]);
    let notes: Vec<&str> = show["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["note"].as_str())
        .collect();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(
        notes[0],
        format!(
            "while running tests\n\nMerged from {source}: guard says too complex to verify\n\
             workaround: run the commands one by one\nhypothesis: the guard cannot parse &&\n\
             subject: guard"
        )
    );
    assert_eq!(
        notes[1],
        format!("Merged from {other}: guard blocked cd && git")
    );

    let stub = flis.sql().query_row(
        "SELECT merged_into, symptom FROM entries WHERE id = ?1",
        [source],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    );
    assert_eq!(
        stub.unwrap(),
        (target, "guard says too complex to verify".to_owned())
    );

    let list = flis.json(&["list"]);
    let listed: Vec<i64> = list["ranked"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    assert_eq!(listed, [target]);
    assert_eq!(list["ranked"]["entries"][0]["projects"], 4);

    let search = flis.json(&["search", "too complex to verify"]);
    let hits = search["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], target);

    let show = flis.json(&["show", &source.to_string()]);
    assert_eq!(show["entry"]["id"], target);
    assert_eq!(show["merged_from"], serde_json::json!([source]));

    let seen = flis.json(&["seen", &other.to_string()]);
    assert_eq!(seen["entry"]["id"], target);
    assert_eq!(occurrence_count(&flis, target), 5);

    let text = flis
        .run(&["merge", &source.to_string(), "--into", &target.to_string()])
        .ok()
        .stdout;
    assert!(
        text.contains(&format!("{source} was already merged into {target}.")),
        "{text}"
    );
    assert!(text.contains("Nothing to merge"), "{text}");
}

#[test]
fn merge_refuses_itself_and_unknown_ids_without_writing() {
    let flis = Flis::new();
    let a = flis.add(&["first"]).to_string();
    let b = flis.add(&["second"]).to_string();

    let run = flis.run(&["merge", &a, "--into", &a]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("into itself"), "{}", run.stderr);

    assert_eq!(flis.run(&["merge", &a, "99", "--into", &b]).code, 3);
    assert_eq!(flis.run(&["merge", &a, "--into", "99"]).code, 3);
    assert_eq!(flis.run(&["merge", "--into", &b]).code, 2);
    assert_eq!(occurrence_count(&flis, 1), 1);
    assert_eq!(occurrence_count(&flis, 2), 1);
    assert!(
        flis.json(&["show", &a])["merged_from"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn merge_into_a_stub_follows_it_and_chains_resolve() {
    let flis = Flis::new();
    let a = flis.add(&["alpha failure"]).to_string();
    let b = flis.add(&["beta failure"]).to_string();
    let c = flis.add(&["gamma failure"]).to_string();
    let d = flis.add(&["delta failure"]).to_string();

    flis.run(&["merge", &a, "--into", &b]).ok();
    let report = flis.json(&["merge", &b, "--into", &c]);
    assert_eq!(report["merged"][0]["occurrences"], 2);

    let show = flis.json(&["show", &a]);
    assert_eq!(show["entry"]["id"].to_string(), c);
    assert_eq!(show["merged_from"], serde_json::json!([1, 2]));

    let report = flis.json(&["merge", &d, "--into", &a]);
    assert_eq!(report["entry"]["id"].to_string(), c);
    assert_eq!(report["merged_from"], serde_json::json!([1, 2]));
    assert_eq!(report["seen"]["count"], 4);

    let run = flis.run(&["merge", &c, "--into", &a]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains(&format!("{a} was merged into {c}")),
        "{}",
        run.stderr
    );

    let text = flis.run(&["list"]).ok().stdout;
    assert!(text.contains("gamma failure"), "{text}");
    for gone in ["alpha", "beta", "delta"] {
        assert!(!text.contains(gone), "{text}");
    }
}

#[test]
fn split_moves_named_occurrences_into_a_new_entry() {
    let mut flis = Flis::new();
    let id = flis.add(&[
        "push rejected",
        "--subject",
        "git",
        "--workaround",
        "pull first",
        "--project",
        "acme",
    ]);
    flis.at("2026-11-14T13:00:00Z");
    for project in ["nova", "shop"] {
        flis.run(&[
            "seen",
            &id.to_string(),
            "--project",
            project,
            "--note",
            "hook said no",
        ])
        .ok();
    }
    flis.at("2026-11-14T14:00:00Z");
    let report = flis.json(&[
        "split",
        &id.to_string(),
        "--occurrence",
        "o2",
        "3",
        "--symptom",
        "pre-push hook rejected the push",
    ]);
    let new_id = report["id"].as_i64().unwrap();
    assert_eq!(report["moved"], serde_json::json!([2, 3]));
    assert_eq!(report["seen"]["count"], 2);
    assert_eq!(report["source_seen"]["count"], 1);
    assert_eq!(report["entry"]["subject"], "git");
    assert!(report["entry"]["workaround"].is_null());
    assert_eq!(report["entry"]["created_at"], "2026-11-14T14:00:00Z");

    let show = flis.json(&["show", &new_id.to_string()]);
    let projects: Vec<&str> = show["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["project"].as_str().unwrap())
        .collect();
    assert_eq!(projects, ["nova", "shop"]);
    assert_eq!(occurrence_count(&flis, id), 1);

    let search = flis.json(&["search", "hook said no"]);
    assert_eq!(search["hits"][0]["id"], new_id);
    assert_eq!(search["hits"].as_array().unwrap().len(), 1);

    let text = flis
        .run(&[
            "split",
            &new_id.to_string(),
            "--occurrence",
            "o3",
            "--symptom",
            "x",
        ])
        .ok()
        .stdout;
    assert!(
        text.starts_with(&format!(
            "Split o3 from {new_id} into new entry {}. {new_id} keeps 1 occurrence.",
            new_id + 1
        )),
        "{text}"
    );
}

#[test]
fn split_refuses_to_empty_an_entry_or_take_foreign_occurrences() {
    let flis = Flis::new();
    let a = flis.add(&["first"]).to_string();
    flis.run(&["seen", &a]).ok();
    flis.add(&["second"]);

    let run = flis.run(&["split", &a, "--occurrence", "o1", "o2", "--symptom", "x"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("every occurrence"), "{}", run.stderr);

    let run = flis.run(&["split", &a, "--occurrence", "o3", "--symptom", "x"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("belongs to entry 2"), "{}", run.stderr);

    assert_eq!(
        flis.run(&["split", &a, "--occurrence", "o99", "--symptom", "x"])
            .code,
        3
    );
    assert_eq!(flis.run(&["split", &a, "--occurrence", "o1"]).code, 2);
    assert_eq!(flis.run(&["split", &a, "--symptom", "x"]).code, 2);
    assert_eq!(
        flis.run(&["split", &a, "--occurrence", "p1", "--symptom", "x"])
            .code,
        2
    );
    assert_eq!(
        flis.run(&["split", &a, "--occurrence", "o1", "--symptom", " "])
            .code,
        2
    );
    assert_eq!(occurrence_count(&flis, 1), 2);
    assert_eq!(
        flis.json(&["list"])["ranked"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn split_repairs_a_wrong_merge() {
    let flis = Flis::new();
    let a = flis
        .add(&["npm install hangs", "--project", "acme"])
        .to_string();
    let b = flis
        .add(&["npm install hangs on a proxy", "--project", "nova"])
        .to_string();
    flis.run(&["merge", &b, "--into", &a]).ok();
    let report = flis.json(&[
        "split",
        &b,
        "--occurrence",
        "o2",
        "--symptom",
        "npm install hangs behind the proxy",
    ]);
    assert_eq!(report["source"]["id"].to_string(), a);
    assert_eq!(report["merged_from"], serde_json::json!([2]));
    let note = flis.json(&["show", &report["id"].to_string()])["occurrences"][0]["note"].clone();
    assert_eq!(note, "Merged from 2: npm install hangs on a proxy");
}
