use serde_json::json;

use crate::common::Flis;

fn fixture() -> String {
    format!("{}/tests/fixtures/old-log.md", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn dry_run_reports_and_writes_nothing() {
    let flis = Flis::new();
    let report = flis.json(&["import", &fixture(), "--dry-run"]);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["header"], true);
    assert_eq!(report["entries"], 6);
    assert_eq!(report["imported"], 0);
    assert_eq!(report["lines"], 12);
    assert_eq!(report["detail_lines"], 3);
    assert_eq!(report["projects_as_written"], 6);
    assert_eq!(
        report["projects"],
        json!([
            {"name": null, "entries": 2},
            {"name": "acme", "entries": 1},
            {"name": "nova", "entries": 1},
            {"name": "old-name", "entries": 1},
            {"name": "shop", "entries": 1},
        ])
    );
    let malformed: Vec<i64> = report["malformed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["line"].as_i64().unwrap())
        .collect();
    assert_eq!(malformed, [16, 18, 19]);
    assert_eq!(
        report["malformed"][0]["detail"],
        json!(["by hand · and lost a field · mono"])
    );

    assert_eq!(flis.json(&["stats"])["entries"], 0);
    let imports: i64 = flis
        .sql()
        .query_row("SELECT count(*) FROM imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(imports, 0);

    let text = flis.run(&["import", &fixture(), "--dry-run"]).ok().stdout;
    assert!(
        text.starts_with("Dry run: would import 6 entries from "),
        "{text}"
    );
    assert!(
        text.contains("Read 12 lines after the header: 6 entries, 3 detail lines, 3 malformed."),
        "{text}"
    );
    assert!(
        text.contains("Projects after normalisation (5 from 6 as written):"),
        "{text}"
    );
    assert!(text.contains("  (no project)  2\n"), "{text}");
    assert!(
        text.contains(
            "  line 16: 2026-09-06 · this line was wrapped\n    by hand · and lost a field · mono\n"
        ),
        "{text}"
    );
    assert!(text.contains("  line 19: just a stray line\n"), "{text}");
}

#[test]
fn imports_each_line_as_an_open_entry() {
    let mut flis = Flis::new();
    flis.at("2026-11-14T12:00:00Z");
    let report = flis.json(&["import", &fixture()]);
    assert_eq!(report["imported"], 6);
    assert_eq!(report["first_id"], 1);
    assert_eq!(report["last_id"], 6);

    let show = flis.json(&["show", "1"]);
    let entry = &show["entry"];
    assert_eq!(entry["symptom"], "bundle install: Could not find gem rails");
    assert_eq!(entry["workaround"], "run bin/setup first");
    assert_eq!(entry["status"], "open");
    assert_eq!(entry["created_at"], "2026-09-01T00:00:00Z");
    let occurrence = &show["occurrences"][0];
    assert_eq!(occurrence["at"], "2026-09-01T00:00:00Z");
    assert_eq!(occurrence["project"], "acme");
    assert_eq!(occurrence["cost"], "minor");
    assert_eq!(
        occurrence["note"],
        "happened in a fresh worktree\nbin/setup sets BUNDLE_PATH"
    );
    assert!(occurrence["agent"].is_null());
    assert!(occurrence["branch"].is_null());

    let docker = flis.json(&["show", "5"]);
    assert!(docker["entry"]["workaround"].is_null());
    assert!(docker["occurrences"][0]["project"].is_null());

    let search = flis.json(&["search", "too complex to verify"]);
    assert_eq!(search["hits"].as_array().unwrap().len(), 2);
    assert_eq!(flis.json(&["search", "BUNDLE_PATH"])["hits"][0]["id"], 1);

    let list = flis.json(&["list", "--since", "2026-09-01"]);
    assert_eq!(list["ranked"]["entries"].as_array().unwrap().len(), 6);
    let stats = flis.json(&["stats"]);
    assert_eq!(stats["status"]["open"], 6);
}

#[test]
fn a_second_import_is_refused_unless_forced() {
    let flis = Flis::new();
    flis.run(&["import", &fixture()]).ok();
    let run = flis.run(&["import", &fixture()]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("already imported"), "{}", run.stderr);
    assert!(run.stderr.contains("--force"), "{}", run.stderr);
    assert_eq!(flis.json(&["stats"])["entries"], 6);

    let copy = flis.path("copy.md");
    std::fs::copy(fixture(), &copy).unwrap();
    let run = flis.run(&["import", copy.to_str().unwrap(), "--json"]);
    assert_eq!(run.code, 2);
    assert!(
        run.json()["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("this content")
    );

    let grown = flis.path("grown.md");
    std::fs::write(&grown, "---\n2026-10-01 · a · b · acme\n").unwrap();
    let grown = grown.to_str().unwrap();
    flis.run(&["import", grown]).ok();
    std::fs::write(
        flis.path("grown.md"),
        "---\n2026-10-01 · a · b · acme\n2026-10-02 · c · d · nova\n",
    )
    .unwrap();
    let run = flis.run(&["import", grown]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("a file at this path"), "{}", run.stderr);

    let dry = flis.run(&["import", &fixture(), "--dry-run"]).ok().stdout;
    assert!(dry.contains("This content was imported before"), "{dry}");
    assert!(dry.contains("A real import needs --force."), "{dry}");

    let forced = flis.json(&["import", &fixture(), "--force"]);
    assert_eq!(forced["imported"], 6);
    assert_eq!(forced["previous"].as_array().unwrap().len(), 1);
    assert_eq!(flis.json(&["stats"])["entries"], 13);
}

#[test]
fn project_map_renames_after_normalisation() {
    let flis = Flis::new();
    let map = flis.path("map.json");
    std::fs::write(&map, r#"{"old-name": "nova", "shop": null, "never": "x"}"#).unwrap();
    let report = flis.json(&["import", &fixture(), "--project-map", map.to_str().unwrap()]);
    assert_eq!(
        report["projects"],
        json!([
            {"name": null, "entries": 3},
            {"name": "nova", "entries": 2},
            {"name": "acme", "entries": 1},
        ])
    );
    assert_eq!(report["unused_map_keys"], json!(["never"]));
    assert_eq!(
        flis.json(&["show", "6"])["occurrences"][0]["project"],
        "nova"
    );

    std::fs::write(&map, r#"["not", "an", "object"]"#).unwrap();
    let run = flis.run(&[
        "import",
        &fixture(),
        "--force",
        "--project-map",
        map.to_str().unwrap(),
    ]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("JSON object"), "{}", run.stderr);
}

#[test]
fn unreadable_input_is_a_usage_error() {
    let flis = Flis::new();
    assert_eq!(flis.run(&["import", "/nonexistent/log.md"]).code, 2);
    let binary = flis.path("binary.md");
    std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(flis.run(&["import", binary.to_str().unwrap()]).code, 2);
    assert_eq!(flis.run(&["import"]).code, 2);
}

#[test]
fn import_is_one_transaction() {
    let flis = Flis::new();
    flis.run(&["list"]).ok();
    flis.sql()
        .execute_batch(
            "CREATE TRIGGER fail_on_tmux BEFORE INSERT ON entries
             WHEN NEW.symptom LIKE 'tmux%' BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
    let run = flis.run(&["import", &fixture()]);
    assert_eq!(run.code, 1);
    assert_eq!(flis.json(&["stats"])["entries"], 0);
    let imports: i64 = flis
        .sql()
        .query_row("SELECT count(*) FROM imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(imports, 0);
}
