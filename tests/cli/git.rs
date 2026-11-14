use crate::common::{Flis, git};

#[test]
fn worktrees_report_the_main_repository() {
    let flis = Flis::new();
    let home = flis.path("home");
    let repo = flis.path("src/acme");
    std::fs::create_dir_all(repo.join("app/models")).unwrap();
    git(&home, &repo, &["init", "--quiet"]);
    git(
        &home,
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let worktree = flis.path("trees/acme-feature");
    git(
        &home,
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature",
            worktree.to_str().unwrap(),
        ],
    );
    std::fs::create_dir_all(worktree.join("lib/tasks")).unwrap();

    let added = flis
        .run_in(
            &repo.join("app/models"),
            &["add", "rubocop crashes", "--json"],
        )
        .ok()
        .json();
    let id = added["id"].to_string();
    flis.run_in(&worktree.join("lib/tasks"), &["seen", &id])
        .ok();
    flis.run_in(&worktree, &["seen", &id, "--project", "nova"])
        .ok();
    flis.run(&["seen", &id]).ok();

    let show = flis.json(&["show", &id]);
    let occurrences = show["occurrences"].as_array().unwrap();
    let field = |index: usize, name: &str| occurrences[index][name].as_str().map(str::to_owned);
    assert_eq!(field(0, "project").as_deref(), Some("acme"));
    assert_eq!(field(0, "branch").as_deref(), Some("main"));
    assert_eq!(field(0, "cwd").as_deref(), Some("app/models"));
    assert_eq!(field(1, "project").as_deref(), Some("acme"));
    assert_eq!(field(1, "branch").as_deref(), Some("feature"));
    assert_eq!(field(1, "cwd").as_deref(), Some("lib/tasks"));
    assert_eq!(field(2, "project").as_deref(), Some("nova"));
    assert_eq!(field(2, "cwd").as_deref(), Some("."));
    assert_eq!(field(3, "project"), None);
    assert_eq!(field(3, "branch"), None);
    assert_eq!(field(3, "cwd"), None);
}

#[test]
fn detached_head_has_no_branch() {
    let flis = Flis::new();
    let home = flis.path("home");
    let repo = flis.path("shop");
    std::fs::create_dir_all(&repo).unwrap();
    git(&home, &repo, &["init", "--quiet"]);
    git(
        &home,
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    git(&home, &repo, &["checkout", "--quiet", "--detach"]);

    flis.run_in(&repo, &["add", "detached", "--json"]).ok();
    let show = flis.json(&["show", "1"]);
    assert_eq!(show["occurrences"][0]["project"], "shop");
    assert!(show["occurrences"][0]["branch"].is_null());
}
