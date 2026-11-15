use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;

use crate::common::{Snags, collect};

#[test]
fn parallel_adds_all_land() {
    let snags = Snags::new();
    snags.run(&["list"]).ok();
    std::fs::remove_file(&snags.db).unwrap();

    let children: Vec<_> = (0..10)
        .map(|n| {
            snags
                .command(&snags.path("work"))
                .args([
                    "add",
                    &format!("parallel failure number {n}"),
                    "--note",
                    "concurrent",
                    "--json",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let run = collect(child.wait_with_output().unwrap()).ok();
        assert!(run.json()["id"].is_i64());
    }

    let conn = snags.sql();
    let count = |sql: &str| conn.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM entries"), 10);
    assert_eq!(count("SELECT count(*) FROM occurrences"), 10);
    assert_eq!(count("SELECT count(*) FROM entries_fts"), 10);
    assert_eq!(count("SELECT count(DISTINCT id) FROM entries"), 10);
    assert_eq!(
        snags.json(&["search", "parallel failure"])["hits"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}

#[test]
fn snags_db_overrides_the_default_location() {
    let snags = Snags::new();
    snags.run(&["add", "something"]).ok();
    assert!(snags.db.exists());
    assert!(!snags.path("home/.local/share/snags").exists());
    let mode = std::fs::metadata(&snags.db).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn xdg_data_home_is_used_without_snags_db() {
    let snags = Snags::new();
    let output = snags
        .command(&snags.path("work"))
        .env_remove("SNAGS_DB")
        .args(["add", "something"])
        .output()
        .unwrap();
    collect(output).ok();
    let path = snags.path("home/.local/share/snags/snags.sqlite3");
    assert!(path.exists());
    let mode = std::fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn unwritable_database_names_path_and_setting() {
    let snags = Snags::new();
    let locked = snags.path("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    let db = locked.join("snags.sqlite3");
    let output = snags
        .command(&snags.path("work"))
        .env("SNAGS_DB", &db)
        .args(["add", "x"])
        .output()
        .unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    let run = collect(output);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains(&db.display().to_string()),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("sandbox.filesystem.allowWrite"),
        "{}",
        run.stderr
    );
}

#[test]
fn read_only_database_file_names_path_and_setting() {
    let snags = Snags::new();
    snags.run(&["add", "x"]).ok();
    std::fs::set_permissions(&snags.db, std::fs::Permissions::from_mode(0o400)).unwrap();
    let run = snags.run(&["add", "y"]);
    std::fs::set_permissions(&snags.db, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains(&snags.db.display().to_string()),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("sandbox.filesystem.allowWrite"),
        "{}",
        run.stderr
    );
}

#[test]
fn bad_snags_now_is_a_usage_error() {
    let snags = Snags::new();
    let output = snags
        .command(&snags.path("work"))
        .env("SNAGS_NOW", "tomorrow")
        .args(["list"])
        .output()
        .unwrap();
    assert_eq!(collect(output).code, 2);
}
