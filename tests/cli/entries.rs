use crate::common::Snags;

#[test]
fn add_then_show_round_trips_every_field() {
    let snags = Snags::new();
    let report = snags.json(&[
        "add",
        "cargo",
        "build",
        "fails:",
        "linker `cc` not found",
        "--workaround",
        "xcode-select --install",
        "--hypothesis",
        "command line tools removed by an OS update",
        "--fix",
        "check for them in bootstrap",
        "--subject",
        "Xcode",
        "--silent",
        "--cost",
        "major",
        "--note",
        "after the upgrade",
        "--project",
        "acme",
        "--agent",
        "codex",
    ]);
    let id = report["id"].as_i64().unwrap();
    assert_eq!(report["occurrence_id"], 1);

    let show = snags.json(&["show", &id.to_string()]);
    let entry = &show["entry"];
    assert_eq!(entry["symptom"], "cargo build fails: linker `cc` not found");
    assert_eq!(entry["workaround"], "xcode-select --install");
    assert_eq!(
        entry["hypothesis"],
        "command line tools removed by an OS update"
    );
    assert_eq!(entry["fix"], "check for them in bootstrap");
    assert_eq!(entry["subject"], "xcode");
    assert_eq!(entry["silent"], true);
    assert_eq!(entry["status"], "open");
    assert_eq!(entry["created_at"], "2026-11-14T12:00:00Z");
    let occurrence = &show["occurrences"][0];
    assert_eq!(occurrence["cost"], "major");
    assert_eq!(occurrence["note"], "after the upgrade");
    assert_eq!(occurrence["project"], "acme");
    assert_eq!(occurrence["agent"], "codex");
    assert_eq!(occurrence["at"], "2026-11-14T12:00:00Z");

    let text = snags.run(&["show", &id.to_string()]).ok().stdout;
    assert!(
        text.starts_with(&format!("{id} open · xcode · SILENT\n")),
        "{text}"
    );
    assert!(
        text.contains("o1 2026-11-14T12:00:00Z major acme codex"),
        "{text}"
    );
}

#[test]
fn agent_and_session_come_from_the_environment() {
    let snags = Snags::new();
    let output = snags
        .command(&snags.path("work"))
        .args(["add", "nova deploy hung", "--json"])
        .env("SNAGS_AGENT", "pi")
        .env("CLAUDE_CODE_SESSION_ID", "session-42")
        .output()
        .unwrap();
    assert!(output.status.success());
    let show = snags.json(&["show", "1"]);
    assert_eq!(show["occurrences"][0]["agent"], "pi");
    assert_eq!(show["occurrences"][0]["session"], "session-42");

    snags.run(&["seen", "1"]).ok();
    let show = snags.json(&["show", "1"]);
    assert_eq!(show["occurrences"][1]["agent"], "claude-code");
    assert!(show["occurrences"][1]["session"].is_null());
}

#[test]
fn input_file_carries_quote_heavy_text() {
    let snags = Snags::new();
    let input = snags.path("entry.json");
    std::fs::write(
        &input,
        r#"{"symptom": "rake aborted! \"can't\" `parse` $HOME 'x'", "workaround": "use \"quotes\"", "cost": "blocker", "silent": true}"#,
    )
    .unwrap();
    let path = input.to_str().unwrap();
    let id = snags.add(&["--input", path, "--workaround", "from the command line"]);
    let show = snags.json(&["show", &id.to_string()]);
    assert_eq!(
        show["entry"]["symptom"],
        r#"rake aborted! "can't" `parse` $HOME 'x'"#
    );
    assert_eq!(show["entry"]["workaround"], "from the command line");
    assert_eq!(show["entry"]["silent"], true);
    assert_eq!(show["occurrences"][0]["cost"], "blocker");

    std::fs::write(&input, r#"{"symptom": "x", "workaroud": "typo"}"#).unwrap();
    let run = snags.run(&["add", "--input", path]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("workaroud"), "{}", run.stderr);
}

#[test]
fn add_lists_similar_entries_and_close_subjects() {
    let snags = Snags::new();
    let first = snags.add(&[
        "bundle install cannot find gem rails",
        "--subject",
        "bundler",
    ]);
    snags.add(&["gh auth token expired", "--subject", "gh"]);

    let report = snags.json(&["add", "bundle exec cannot find gem", "--subject", "bundlr"]);
    assert_eq!(report["similar"][0]["id"], first);
    assert_eq!(report["new_subject"]["closest"][0], "bundler");

    let text = snags
        .run(&["add", "rails gem cannot be found", "--subject", "bundle"])
        .ok()
        .stdout;
    assert!(text.contains("Similar entries"), "{text}");
    assert!(
        text.contains("To switch: snags edit 4 --subject bundler"),
        "{text}"
    );

    let text = snags
        .run(&["add", "docker daemon down", "--subject", "docker"])
        .ok()
        .stdout;
    assert!(
        text.contains("None of the existing subjects is close"),
        "{text}"
    );

    let text = snags
        .run(&["add", "gh pr view slow", "--subject", "gh"])
        .ok()
        .stdout;
    assert!(!text.contains("is new"), "{text}");
}

#[test]
fn subject_must_be_one_word() {
    let snags = Snags::new();
    assert_eq!(snags.run(&["add", "x", "--subject", "two words"]).code, 2);
    assert_eq!(snags.run(&["add"]).code, 2);
    assert_eq!(snags.run(&["add", "   "]).code, 2);
}

#[test]
fn edit_keeps_history_and_only_touches_open_entries() {
    let mut snags = Snags::new();
    let id = snags
        .add(&["tmux pane froze", "--workaround", "restart it"])
        .to_string();
    snags.at("2026-11-14T13:00:00Z");
    let report = snags.json(&[
        "edit",
        &id,
        "--workaround",
        "send SIGCONT",
        "--silent",
        "--ref",
        "acme/tools#7",
    ]);
    assert_eq!(
        report["changed"],
        serde_json::json!(["workaround", "ref", "silent"])
    );

    let same = snags.json(&["edit", &id, "--workaround", "send SIGCONT"]);
    assert_eq!(same["changed"], serde_json::json!([]));

    let show = snags.json(&["show", &id, "--history"]);
    let history = show["history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["field"], "workaround");
    assert_eq!(history[0]["old_value"], "restart it");
    assert_eq!(history[0]["at"], "2026-11-14T13:00:00Z");
    assert_eq!(show["entry"]["updated_at"], "2026-11-14T13:00:00Z");

    let text = snags.run(&["show", &id, "--history"]).ok().stdout;
    assert!(
        text.contains("2026-11-14T13:00:00Z workaround: restart it"),
        "{text}"
    );

    assert_eq!(snags.run(&["edit", &id]).code, 2);
    assert_eq!(snags.run(&["edit", &id, "--symptom", ""]).code, 2);
    snags.run(&["edit", &id, "--ref", ""]).ok();
    assert!(snags.json(&["show", &id])["entry"]["ref"].is_null());

    snags
        .run(&[
            "resolve",
            &id,
            "--where",
            "upstream",
            "--ref",
            "https://example.com/7",
        ])
        .ok();
    let run = snags.run(&["edit", &id, "--fix", "x"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("snags reopen"), "{}", run.stderr);
}

#[test]
fn resolve_requires_where_and_ref() {
    let snags = Snags::new();
    let id = snags.add(&["something"]).to_string();
    assert_eq!(snags.run(&["resolve", &id, "--ref", "x"]).code, 2);
    assert_eq!(snags.run(&["resolve", &id, "--where", "setup"]).code, 2);
    assert_eq!(
        snags
            .run(&["resolve", &id, "--where", "none", "--ref", "x"])
            .code,
        2
    );
    assert_eq!(
        snags
            .run(&["resolve", &id, "--where", "setup", "--ref", " "])
            .code,
        2
    );
    snags
        .run(&["resolve", &id, "--where", "project", "--ref", "abc123"])
        .ok();
    assert_eq!(
        snags
            .run(&["resolve", &id, "--where", "project", "--ref", "abc123"])
            .code,
        2
    );
}

#[test]
fn seen_on_resolved_entry_is_a_regression() {
    let mut snags = Snags::new();
    let id = snags
        .add(&[
            "worktree guard refuses compound command",
            "--workaround",
            "split it",
        ])
        .to_string();
    snags.at("2026-11-14T13:00:00Z");
    let resolved = snags.json(&[
        "resolve",
        &id,
        "--where",
        "setup",
        "--ref",
        "dotfiles@1a2b",
        "--note",
        "allowlisted",
    ]);
    assert_eq!(resolved["entry"]["status"], "resolved");
    assert_eq!(resolved["entry"]["resolved_at"], "2026-11-14T13:00:00Z");

    snags.at("2026-11-15T09:00:00Z");
    let seen = snags.json(&["seen", &id, "--note", "came back", "--cost", "major"]);
    assert_eq!(seen["entry"]["status"], "open");
    assert_eq!(seen["entry"]["regressions"], 1);
    assert!(seen["entry"]["ref"].is_null());
    assert_eq!(seen["regression"]["ref"], "dotfiles@1a2b");
    assert_eq!(seen["seen"]["count"], 2);

    let show = snags.json(&["show", &id, "--history"]);
    let fields: Vec<_> = show["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["field"].as_str().unwrap().to_owned())
        .collect();
    assert!(fields.contains(&"ref".to_owned()), "{fields:?}");
    let old_ref = show["history"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|h| h["field"] == "ref")
        .unwrap();
    assert_eq!(old_ref["old_value"], "dotfiles@1a2b");

    snags
        .run(&["resolve", &id, "--where", "setup", "--ref", "dotfiles@3c4d"])
        .ok();
    let text = snags.run(&["seen", &id]).ok().stdout;
    assert!(text.contains("Regression 2"), "{text}");
    assert!(text.contains("workaround: split it"), "{text}");
}

#[test]
fn seen_on_accepted_entry_keeps_it_accepted() {
    let snags = Snags::new();
    let id = snags.add(&["macOS asks for keychain access"]);
    snags
        .sql()
        .execute(
            "UPDATE entries SET status = 'accepted', resolution_note = 'by design' WHERE id = ?1",
            [id],
        )
        .unwrap();
    let seen = snags.json(&["seen", &id.to_string()]);
    assert_eq!(seen["entry"]["status"], "accepted");
    assert!(seen["regression"].is_null());
}

#[test]
fn reopen_clears_the_resolution() {
    let snags = Snags::new();
    let id = snags.add(&["ssh agent missing"]).to_string();
    let already = snags.run(&["reopen", &id]).ok().stdout;
    assert!(already.contains("already open"), "{already}");
    snags
        .run(&["resolve", &id, "--where", "setup", "--ref", "abc"])
        .ok();
    let report = snags.json(&["reopen", &id]);
    assert_eq!(report["entry"]["status"], "open");
    assert_eq!(report["previous_status"], "resolved");
    assert!(report["entry"]["ref"].is_null());
    assert_eq!(report["entry"]["regressions"], 0);
}

#[test]
fn merged_ids_lead_to_the_surviving_entry() {
    let snags = Snags::new();
    let source = snags.add(&["guard refused cd && git"]);
    let target = snags.add(&["guard refused compound command"]);
    snags
        .sql()
        .execute(
            "UPDATE entries SET merged_into = ?1 WHERE id = ?2",
            [target, source],
        )
        .unwrap();

    let seen = snags.json(&["seen", &source.to_string(), "--note", "via old id"]);
    assert_eq!(seen["entry"]["id"], target);
    assert_eq!(seen["merged_from"], serde_json::json!([source]));

    let show = snags.json(&["show", &source.to_string()]);
    assert_eq!(show["entry"]["id"], target);
    assert_eq!(show["occurrences"].as_array().unwrap().len(), 2);
    let text = snags.run(&["show", &source.to_string()]).ok().stdout;
    assert!(
        text.starts_with(&format!("{source} was merged into {target}.\n")),
        "{text}"
    );

    let search = snags.json(&["search", "guard refused"]);
    assert_eq!(search["hits"].as_array().unwrap().len(), 1);
    assert_eq!(search["hits"][0]["id"], target);
    let list = snags.json(&["list"]);
    assert_eq!(list["ranked"]["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn exit_codes() {
    let snags = Snags::new();
    let missing = snags.run(&["show", "99"]);
    assert_eq!(missing.code, 3);
    assert!(missing.stderr.contains("no entry 99"), "{}", missing.stderr);
    assert_eq!(snags.run(&["seen", "99"]).code, 3);
    assert_eq!(
        snags
            .run(&["resolve", "99", "--where", "setup", "--ref", "x"])
            .code,
        3
    );
    assert_eq!(snags.run(&["show", "abc"]).code, 2);
    assert_eq!(snags.run(&["add", "x", "--cost", "huge"]).code, 2);
    assert_eq!(snags.run(&["frobnicate"]).code, 2);
    assert_eq!(snags.run(&[]).code, 2);
    assert_eq!(snags.run(&["--help"]).code, 0);
    assert_eq!(snags.run(&["list", "--since", "soon"]).code, 2);
}

#[test]
fn json_errors_are_one_object_on_stdout() {
    let snags = Snags::new();
    let missing = snags.run(&["show", "99", "--json"]);
    assert_eq!(missing.code, 3);
    assert!(missing.stderr.is_empty());
    assert_eq!(missing.json()["error"]["kind"], "not_found");

    let usage = snags.run(&["add", "x", "--cost", "huge", "--json"]);
    assert_eq!(usage.code, 2);
    assert!(usage.stderr.is_empty(), "{}", usage.stderr);
    assert_eq!(usage.json()["error"]["kind"], "usage");
}

#[test]
fn version_and_help() {
    let snags = Snags::new();
    let version = snags.run(&["--version"]).ok().stdout;
    assert_eq!(
        version.trim(),
        format!("snags {}", env!("CARGO_PKG_VERSION"))
    );
    let help = snags.run(&["add", "--help"]).ok().stdout;
    for field in [
        "--workaround",
        "--hypothesis",
        "--fix",
        "--subject",
        "--silent",
        "--cost",
        "--note",
        "--input",
    ] {
        assert!(help.contains(field), "{field}");
    }
    assert!(help.contains("Do not log bugs in the project"));
}
