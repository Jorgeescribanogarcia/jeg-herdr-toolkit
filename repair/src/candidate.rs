//! Looking for where a repo folder went: a sibling folder with a similar name that is a git repo with
//! the same remote. Only a single match is ever proposed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The one folder that fits, with the same remote.
    Unique(PathBuf),
    /// The only similar-named git repo, but with no remote to compare (no surviving checkout to read
    /// it from): the name is all there is, so whoever repairs has to confirm it.
    Unverified(PathBuf),
    /// Nothing to propose, and why.
    None(String),
    /// Several fit: guessing would be wrong, so none is proposed.
    Ambiguous(Vec<PathBuf>),
}

/// `origin` of the repo at `dir`, normalised so that `https://host/o/r.git` and `https://host/o/r` match.
pub fn remote_of(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_lowercase();
    let url = url.trim_end_matches('/').trim_end_matches(".git").to_string();
    (output.status.success() && !url.is_empty()).then_some(url)
}

fn plain(path: &str) -> String {
    path.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Whether the repo at `dir` lists one of `checkouts` as a linked worktree. After a rename this is
/// often the only proof left: the checkout's own link to the repo is broken, so git cannot read its
/// remote, but the repo still remembers the checkout.
fn lists_a_checkout(dir: &Path, checkouts: &[String]) -> bool {
    let Ok(entries) = fs::read_dir(dir.join(".git").join("worktrees")) else { return false };
    entries.filter_map(Result::ok).filter_map(|e| fs::read_to_string(e.path().join("gitdir")).ok()).any(|link| {
        let link = plain(&link);
        let link = link.strip_suffix("\\.git").unwrap_or(&link);
        checkouts.iter().any(|c| plain(c) == link)
    })
}

/// Case, spaces, dashes and underscores do not count: `My App` and `my-app` are the same name.
fn simplified(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != cb)).min(row[j] + 1).min(above + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

fn similar_names(a: &str, b: &str) -> bool {
    let (a, b) = (simplified(a), simplified(b));
    edit_distance(&a, &b) <= 2.max(a.len().max(b.len()) / 5)
}

/// Looks next to `old` (the folder that no longer exists) for its new place. `checkouts` are existing
/// checkouts of the same repo: the remote is read from them, since the repo itself is gone. With `repo`
/// false the folder is a plain one and any similarly named folder fits.
pub fn find(old: &Path, checkouts: &[String], repo: bool) -> Outcome {
    let remote = checkouts.iter().find_map(|c| remote_of(Path::new(c)));
    let (Some(parent), Some(name)) = (old.parent(), old.file_name()) else {
        return Outcome::None(t!(
            "the folder has no parent to look in",
            "la carpeta no tiene carpeta superior donde buscar"
        ));
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return Outcome::None(t!("{} no longer exists", "{} ya no existe", parent.display()));
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            (if repo { p.join(".git").exists() } else { p.is_dir() })
                && p.file_name().is_some_and(|n| similar_names(&name.to_string_lossy(), &n.to_string_lossy()))
        })
        .filter(|p| {
            checkouts.is_empty() || remote_of(p).is_some_and(|r| Some(r) == remote) || lists_a_checkout(p, checkouts)
        })
        .collect();
    found.sort();
    match (found.len(), checkouts.is_empty()) {
        (0, _) => Outcome::None(t!(
            "no sibling folder with a similar name is a git repo of this project",
            "ninguna carpeta hermana con nombre parecido es un repo git de este proyecto"
        )),
        (1, false) => Outcome::Unique(found.remove(0)),
        (1, true) => Outcome::Unverified(found.remove(0)),
        _ => Outcome::Ambiguous(found),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo(parent: &Path, name: &str, remote: Option<&str>) -> PathBuf {
        let dir = parent.join(name);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        if let Some(url) = remote {
            git(&dir, &["remote", "add", "origin", url]);
        }
        dir
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-repair-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_that_differ_by_a_typo_are_similar() {
        assert!(similar_names("Notes Appp", "Notes App"));
        assert!(similar_names("my-app", "My App"));
        assert!(!similar_names("app", "website"));
    }

    #[test]
    fn one_sibling_with_the_same_remote_is_proposed() {
        let base = scratch("unique");
        let survivor = repo(&base, "wt-source", Some("https://example.com/alice/app.git"));
        let new = repo(&base, "Notes App", Some("https://example.com/alice/app"));
        repo(&base, "Notes Other", Some("https://example.com/alice/other.git"));
        let outcome = find(&base.join("Notes Appp"), &[survivor.to_string_lossy().into_owned()], true);
        assert_eq!(outcome, Outcome::Unique(new));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn two_matching_siblings_are_not_guessed() {
        let base = scratch("ambiguous");
        let survivor = repo(&base, "wt-source", Some("https://example.com/alice/app.git"));
        repo(&base, "Notes App", Some("https://example.com/alice/app.git"));
        repo(&base, "Notes App2", Some("https://example.com/alice/app.git"));
        let outcome = find(&base.join("Notes Appp"), &[survivor.to_string_lossy().into_owned()], true);
        assert!(matches!(outcome, Outcome::Ambiguous(found) if found.len() == 2));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_renamed_repo_is_recognised_by_the_checkout_it_remembers() {
        let base = scratch("linked");
        let old = repo(&base, "Notes Appp", None);
        git(&old, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "init"]);
        let checkout = base.join("checkouts").join("feature");
        git(&old, &["worktree", "add", "-q", checkout.to_str().unwrap()]);
        let new = base.join("Notes App");
        fs::rename(&old, &new).unwrap();
        repo(&base, "Notes App-2", None);
        assert_eq!(find(&old, &[checkout.to_string_lossy().into_owned()], true), Outcome::Unique(new));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_similar_folder_with_another_remote_is_not_proposed() {
        let base = scratch("remote");
        let survivor = repo(&base, "wt-source", Some("https://example.com/alice/app.git"));
        repo(&base, "Notes App", Some("https://example.com/bob/app.git"));
        repo(&base, "Notes App-notes", None);
        let outcome = find(&base.join("Notes Appp"), &[survivor.to_string_lossy().into_owned()], true);
        assert!(matches!(outcome, Outcome::None(_)));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn without_a_surviving_checkout_the_name_alone_is_unverified() {
        let base = scratch("nocheckout");
        let new = repo(&base, "Notes App", Some("https://example.com/alice/app.git"));
        repo(&base, "Unrelated", None);
        assert_eq!(find(&base.join("Notes Appp"), &[], true), Outcome::Unverified(new));
        repo(&base, "Notes App-2", None);
        assert!(matches!(find(&base.join("Notes Appp"), &[], true), Outcome::Ambiguous(_)));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_plain_folder_needs_no_git_repo() {
        let base = scratch("plain");
        let new = base.join("Notes App");
        fs::create_dir_all(&new).unwrap();
        fs::create_dir_all(base.join("Unrelated")).unwrap();
        assert_eq!(find(&base.join("Notes Appp"), &[], false), Outcome::Unverified(new));
        assert!(matches!(find(&base.join("Notes Appp"), &[], true), Outcome::None(_)));
        let _ = fs::remove_dir_all(&base);
    }
}
