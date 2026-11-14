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
