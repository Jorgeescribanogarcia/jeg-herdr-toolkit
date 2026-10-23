//! Installing, updating and uninstalling from the explorer, through herdr's own commands; and the
//! manifest of a listed plugin, to show what it runs before it is installed.

use std::process::{Command, Stdio};

use crate::http;
use crate::index::Listing;
use crate::installed::Manifest;
use crate::process;
use crate::text::{clean, valid_repo, valid_subdir};
use crate::ui_updates::{Status, outcome};
use crate::updates::parse_ls_remote;

/// The manifest at `commit`, from raw.githubusercontent.com (no API quota). `None` if the repo, path or
/// commit are not safe to put in a URL.
pub fn manifest_url_at(l: &Listing, commit: &str) -> Option<String> {
    let commit_ok = !commit.is_empty() && commit.chars().all(|c| c.is_ascii_alphanumeric());
    (valid_repo(&l.repo) && commit_ok && valid_subdir(&l.path))
        .then(|| format!("https://raw.githubusercontent.com/{}/{commit}/{}", l.repo, l.path))
}

/// The manifest herdr would install now: at the commit the default branch points to (the index can be
/// 45 minutes old), falling back to the index's commit if that cannot be asked. What is shown can
/// still differ if the author pushes in the seconds between this and the install.
pub fn fetch_manifest(l: &Listing) -> Result<Manifest, String> {
    if !valid_repo(&l.repo) {
        return Err("the index gives a repo name that is not safe to use".into());
    }
    let live = process::git_ls_remote(&l.repo).ok().and_then(|refs| parse_ls_remote(&refs, None));
    let commit = live.as_deref().unwrap_or(&l.head_commit);
    let url = manifest_url_at(l, commit).ok_or("the index gives a path that is not safe to use")?;
    Manifest::parse(&http::get(&url)?)
}

/// `herdr plugin install owner/repo[/folder] --yes`, once the names are checked: they come from a
/// third-party index and end up on a command line.
pub fn install_args(l: &Listing) -> Result<Vec<String>, String> {
    let dir_ok = l.path.rsplit_once('/').is_none_or(|(dir, _)| valid_subdir(dir));
    if !valid_repo(&l.repo) || !dir_ok {
        return Err(format!("“{}” is not a repo name that can be installed safely", clean(&l.repo)));
    }
    Ok(vec!["plugin".into(), "install".into(), l.install_source(), "--yes".into()])
}

pub fn update_args(id: &str) -> Vec<String> {
    vec!["plugin".into(), "update".into(), id.into(), "--yes".into()]
}

/// herdr's uninstall takes no `--yes`; it runs with stdin closed, so it cannot wait for an answer.
pub fn uninstall_args(id: &str) -> Vec<String> {
    vec!["plugin".into(), "uninstall".into(), id.into()]
}

/// Runs one of the commands above and says how it went. herdr does not stream the build, so there is
/// nothing to show until it finishes.
pub fn run(herdr: &str, args: &[String]) -> Status {
    match Command::new(herdr).args(args).stdin(Stdio::null()).output() {
        Ok(o) => outcome(o.status.success(), &String::from_utf8_lossy(&o.stdout), &String::from_utf8_lossy(&o.stderr)),
        Err(e) => Status::Failed(format!("could not run herdr: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::parse;

    fn listing(id: &str) -> Listing {
        parse(include_str!("testdata/index.json")).unwrap().into_iter().find(|l| l.id == id).unwrap()
    }

    #[test]
    fn manifest_url_uses_commit_and_path() {
        let links = listing("annotate-links");
        assert_eq!(
            manifest_url_at(&links, &links.head_commit).unwrap(),
            format!(
                "https://raw.githubusercontent.com/plannotator/herdr-annotate/{}/links/herdr-plugin.toml",
                links.head_commit
            )
        );
    }

    #[test]
    fn manifest_url_at_the_live_commit() {
        let links = listing("annotate-links");
        assert_eq!(
            manifest_url_at(&links, "abc123").unwrap(),
            "https://raw.githubusercontent.com/plannotator/herdr-annotate/abc123/links/herdr-plugin.toml"
        );
        assert!(manifest_url_at(&links, "abc/../x").is_none());
    }

    #[test]
    fn refuses_invalid_repo() {
        let mut l = listing("annotate");
        l.repo = "-evil/repo".into();
        assert!(manifest_url_at(&l, &l.head_commit).is_none());
        assert!(install_args(&l).is_err());
        l.repo = "plannotator/herdr-annotate".into();
        l.path = "../../x/herdr-plugin.toml".into();
        assert!(install_args(&l).is_err());
        l.path = "links/../../herdr-plugin.toml".into();
        assert!(manifest_url_at(&l, &l.head_commit).is_none());
    }

    #[test]
    fn error_does_not_echo_control_characters() {
        let mut l = listing("annotate");
        l.repo = "\u{1b}[2J/x".into();
        let error = install_args(&l).unwrap_err();
        assert!(!error.contains('\u{1b}'), "{error:?}");
    }

    #[test]
    fn install_arguments() {
        assert_eq!(
            install_args(&listing("annotate-links")).unwrap(),
            ["plugin", "install", "plannotator/herdr-annotate/links", "--yes"]
        );
        assert_eq!(update_args("annotate"), ["plugin", "update", "annotate", "--yes"]);
        assert_eq!(uninstall_args("annotate"), ["plugin", "uninstall", "annotate"]);
    }
}
