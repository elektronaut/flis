use crate::common::Snags;

const HOSTILE: &str = r#"error: can't find gem "foo" (>= 0) -- NEAR/3 AND OR *"#;

#[test]
fn hostile_input_falls_back_to_any_word() {
    let snags = Snags::new();
    let id = snags.add(&[
        "bundler could not find gem foo",
        "--subject",
        "bundler",
        "--workaround",
        "bin/setup",
    ]);
    snags.add(&["gh pr create hangs", "--subject", "gh"]);

    let report = snags.json(&["search", HOSTILE]);
    assert_eq!(report["match"], "any");
    let hits = report["hits"].as_array().unwrap();
    assert_eq!(hits[0]["id"], id);
    assert_eq!(hits.len(), 1);

    let text = snags.run(&["search", HOSTILE]).ok().stdout;
    assert!(text.contains("No entry matches all words"), "{text}");
    assert!(text.contains("workaround: bin/setup"), "{text}");
}

#[test]
fn all_words_match_first() {
    let snags = Snags::new();
    let both = snags.add(&["git push rejected: remote hook declined"]);
    snags.add(&["git fetch timed out"]);

    let report = snags.json(&["search", "git", "push", "declined"]);
    assert_eq!(report["match"], "all");
    let ids: Vec<_> = report["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].clone())
        .collect();
    assert_eq!(ids, [both]);

    let text = snags.run(&["search", "git push declined"]).ok().stdout;
    assert!(text.starts_with("1 matching all words"), "{text}");
}

#[test]
fn no_hits_is_success() {
    let snags = Snags::new();
    snags.add(&["docker pull fails on keychain"]);
    let run = snags.run(&["search", "terraform"]).ok();
    assert_eq!(run.stdout.trim(), "No entries match.");
    assert_eq!(snags.json(&["search", "terraform"])["match"], "none");
}

#[test]
fn only_punctuation_is_a_usage_error() {
    let snags = Snags::new();
    assert_eq!(snags.run(&["search", "-- !!! *"]).code, 2);
}

#[test]
fn occurrence_notes_are_searchable() {
    let snags = Snags::new();
    let id = snags.add(&["psql hangs", "--note", "first seen in migration zanzibar"]);
    snags
        .run(&["seen", &id.to_string(), "--note", "again with quokka flag"])
        .ok();

    for word in ["zanzibar", "quokka"] {
        let report = snags.json(&["search", word]);
        assert_eq!(report["hits"][0]["id"], id, "{word}");
    }
}

#[test]
fn edits_are_searchable_and_old_text_is_not() {
    let snags = Snags::new();
    let id = snags
        .add(&["mise shim missing", "--workaround", "reinstall wombat"])
        .to_string();
    snags
        .run(&["edit", &id, "--workaround", "run mise reshim"])
        .ok();
    assert_eq!(
        snags.json(&["search", "reshim"])["hits"][0]["id"].to_string(),
        id
    );
    assert_eq!(snags.json(&["search", "wombat"])["match"], "none");
}

#[test]
fn status_filter_and_resolved_hits_show_the_fix() {
    let snags = Snags::new();
    let id = snags.add(&["zsh completion broken"]).to_string();
    snags
        .run(&[
            "resolve",
            &id,
            "--where",
            "setup",
            "--ref",
            "dotfiles@abc123",
            "--note",
            "fpath order",
        ])
        .ok();

    assert_eq!(
        snags.json(&["search", "zsh", "--status", "open"])["match"],
        "none"
    );
    let text = snags.run(&["search", "zsh completion"]).ok().stdout;
    assert!(
        text.contains("fixed: in setup · ref dotfiles@abc123 · fpath order"),
        "{text}"
    );
}
