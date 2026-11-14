use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Location {
    pub project: Option<String>,
    pub branch: Option<String>,
    pub cwd: Option<String>,
}

/// The repository around `dir`, as git sees it. Outside a repository, or
/// when git is missing, every field is `None`.
pub fn locate(dir: &Path) -> Location {
    let Some(output) = git(
        dir,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
            "--show-toplevel",
            "--show-prefix",
        ],
    ) else {
        return Location::default();
    };
    let mut lines = output.lines();
    let (Some(common_dir), Some(_toplevel)) = (lines.next(), lines.next()) else {
        return Location::default();
    };
    let prefix = lines.next().unwrap_or("").trim_end_matches('/');
    let branch = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    Location {
        project: project_name(Path::new(common_dir)),
        branch,
        cwd: Some(if prefix.is_empty() {
            ".".to_owned()
        } else {
            prefix.to_owned()
        }),
    }
}

/// The main repository's directory name, given git's common directory:
/// `acme/.git` and `acme.git` are both `acme`.
pub fn project_name(common_dir: &Path) -> Option<String> {
    let name = common_dir.file_name()?.to_str()?;
    let name = if name == ".git" || name == ".bare" {
        common_dir.parent()?.file_name()?.to_str()?
    } else {
        name.strip_suffix(".git")
            .filter(|stem| !stem.is_empty())
            .unwrap_or(name)
    };
    Some(name.to_owned())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_main_repository() {
        assert_eq!(
            project_name(Path::new("/src/acme/.git")).as_deref(),
            Some("acme")
        );
        assert_eq!(
            project_name(Path::new("/src/nova.git")).as_deref(),
            Some("nova")
        );
        assert_eq!(
            project_name(Path::new("/src/shop/.bare")).as_deref(),
            Some("shop")
        );
        assert_eq!(
            project_name(Path::new("/src/mono")).as_deref(),
            Some("mono")
        );
    }
}
