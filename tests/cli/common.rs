use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

pub const NOW: &str = "2026-11-14T12:00:00Z";

/// A flis with its own database, home and working directory in a temp dir.
pub struct Flis {
    pub temp: TempDir,
    pub db: PathBuf,
    pub now: String,
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("not one JSON object ({error}): {}", self.stdout))
    }

    #[track_caller]
    pub fn ok(self) -> Self {
        assert_eq!(
            self.code, 0,
            "stdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self
    }
}

impl Flis {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("data/flis.sqlite3");
        std::fs::create_dir_all(temp.path().join("home")).unwrap();
        std::fs::create_dir_all(temp.path().join("work")).unwrap();
        Self {
            temp,
            db,
            now: NOW.to_owned(),
        }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.temp.path().join(relative)
    }

    pub fn command(&self, dir: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_flis"));
        command
            .current_dir(dir)
            .env("FLIS_DB", &self.db)
            .env("FLIS_NOW", &self.now)
            .env("HOME", self.path("home"))
            .env("XDG_DATA_HOME", self.path("home/.local/share"))
            .env("GIT_CEILING_DIRECTORIES", self.temp.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("FLIS_AGENT")
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
        command
    }

    pub fn run_in(&self, dir: &Path, args: &[&str]) -> Run {
        collect(self.command(dir).args(args).output().unwrap())
    }

    pub fn run(&self, args: &[&str]) -> Run {
        self.run_in(&self.path("work"), args)
    }

    pub fn json(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let run = self.run(&args).ok();
        assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
        run.json()
    }

    pub fn at(&mut self, now: &str) -> &mut Self {
        self.now = now.to_owned();
        self
    }

    /// Adds an entry and returns its id.
    pub fn add(&self, args: &[&str]) -> i64 {
        let mut full = vec!["add"];
        full.extend_from_slice(args);
        self.json(&full)["id"].as_i64().unwrap()
    }

    pub fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.db).unwrap()
    }
}

pub fn collect(output: Output) -> Run {
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Runs git in `dir` with a fixed identity, isolated from the user's config.
pub fn git(home: &Path, dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
