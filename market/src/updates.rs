//! Which installed plugins have a newer commit on GitHub. The installed commit is compared with the
//! one the same ref points to now (`git ls-remote`: no API quota, nothing downloaded), and only the
//! new commit's manifest is fetched, to show its version and what it would run.

use serde::{Deserialize, Serialize};

use crate::http;
use crate::installed::{self, Installed, Manifest};
use crate::process;
use crate::text::{valid_repo, valid_subdir};

/// An available update, as the updates window shows it. Saved to disk between the check and the window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Update {
    pub id: String,
    pub name: String,
    /// `owner/repo`
    pub repo: String,
    /// Installed version, or the installed commit (short) if the version did not change.
    pub from: String,
    pub to: String,
    pub new_commit: String,
    /// The new version builds or starts something different from the installed one.
    pub runs_changed: bool,
    pub new_build: Vec<Vec<String>>,
    pub new_startup: Vec<Vec<String>>,
}

/// GitHub plugins that `herdr plugin update` could move: not pinned to a commit, and with a known commit.
pub fn candidates(list: &[Installed]) -> Vec<&Installed> {
    list.iter().filter(|p| p.repo.is_some() && p.commit.is_some() && !p.pinned()).collect()
}

/// The commit `git_ref` points to in `git ls-remote` output; `None` (the default branch) is `HEAD`.
/// A tag gives `None`: it is a fixed version, like a pinned commit.
pub fn parse_ls_remote(out: &str, git_ref: Option<&str>) -> Option<String> {
    let wanted = match git_ref {
        None => "HEAD".to_string(),
        Some(r) if r.starts_with("refs/heads/") => r.to_string(),
        Some(r) => format!("refs/heads/{r}"),
    };
    out.lines().find_map(|line| {
        let (sha, name) = line.split_once('\t')?;
        (name == wanted).then(|| sha.to_string())
    })
}

/// `None` if the remote commit is the installed one.
pub fn compare(p: &Installed, remote_commit: &str, new_manifest: &str) -> Result<Option<Update>, String> {
    let installed = p.commit.as_deref().unwrap_or_default();
    if remote_commit == installed {
        return Ok(None);
    }
    let new = Manifest::parse(new_manifest)?;
    let (from, to) = if new.version != p.version {
        (p.version.clone(), new.version.clone())
    } else {
        (short(installed), short(remote_commit))
    };
    Ok(Some(Update {
        id: p.id.clone(),
        name: if p.name.is_empty() { p.id.clone() } else { p.name.clone() },
        repo: p.repo.clone().unwrap_or_default(),
        from,
        to,
        new_commit: remote_commit.to_string(),
        runs_changed: new.build != p.build || new.startup != p.startup,
        new_build: new.build,
        new_startup: new.startup,
    }))
}

fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

/// Where the manifest of `commit` is, without spending GitHub API quota. `None` if the repo or folder
/// names are not safe to put in a URL.
pub fn manifest_url(p: &Installed, commit: &str) -> Option<String> {
    let repo = p.repo.as_deref().filter(|r| valid_repo(r))?;
    if commit.is_empty() || !commit.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let dir = match p.subdir.as_deref() {
        Some(d) if valid_subdir(d) => format!("{d}/"),
        Some(_) => return None,
        None => String::new(),
    };
    Some(format!("https://raw.githubusercontent.com/{repo}/{commit}/{dir}herdr-plugin.toml"))
}

/// Every available update. A plugin that cannot be checked (no network, repo gone) is reported on
/// stderr and skipped: one broken plugin must not hide the others' updates.
pub fn find(herdr: &str) -> Result<Vec<Update>, String> {
    let list = installed::list(herdr)?;
    let mut updates = Vec::new();
    for p in candidates(&list) {
        match check_one(p) {
            Ok(Some(u)) => updates.push(u),
            Ok(None) => {}
            Err(e) => eprintln!("herdr-market: could not check {}: {e}", p.id),
        }
    }
    Ok(updates)
}

fn check_one(p: &Installed) -> Result<Option<Update>, String> {
    let repo = p.repo.as_deref().filter(|r| valid_repo(r)).ok_or("not a valid owner/repo")?;
    let refs = process::git_ls_remote(repo)?;
    let Some(remote) = parse_ls_remote(&refs, p.git_ref.as_deref()) else {
        return Ok(None); // a tag, or a branch that no longer exists: nothing herdr could update to
    };
    if Some(remote.as_str()) == p.commit.as_deref() {
        return Ok(None);
    }
    let url = manifest_url(p, &remote).ok_or("unsafe repo or folder name")?;
    compare(p, &remote, &http::get(&url)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::parse_list;

    const LIST: &str = include_str!("testdata/plugin-list.json");
    const SAME: &str = include_str!("testdata/manifest-same.toml");
    const NEW: &str = include_str!("testdata/manifest-new.toml");
    const OLD_COMMIT: &str = "28f11cce5b9ed02cabe8b5c61756e2ed3b7ac4ff";
    const NEW_COMMIT: &str = "97ba479f6ecd577ca5acaad2f32983c852355f9f";

    fn viewer() -> Installed {
        parse_list(LIST).unwrap().into_iter().find(|p| p.id == "herdr-file-viewer").unwrap()
    }

    #[test]
    fn same_commit_is_no_update() {
        assert_eq!(compare(&viewer(), OLD_COMMIT, SAME).unwrap(), None);
    }

    #[test]
    fn version_change_shows_versions() {
        let u = compare(&viewer(), NEW_COMMIT, NEW).unwrap().unwrap();
        assert_eq!((u.from.as_str(), u.to.as_str()), ("1.18.0", "1.19.0"));
        assert_eq!(u.new_commit, NEW_COMMIT);
        assert_eq!(u.repo, "smarzban/herdr-file-viewer");
    }

    #[test]
    fn same_version_shows_short_commits() {
        let u = compare(&viewer(), NEW_COMMIT, SAME).unwrap().unwrap();
        assert_eq!((u.from.as_str(), u.to.as_str()), ("28f11cc", "97ba479"));
        assert!(!u.runs_changed);
    }

    #[test]
    fn changed_build_is_flagged() {
        let u = compare(&viewer(), NEW_COMMIT, NEW).unwrap().unwrap();
        assert!(u.runs_changed);
        assert_eq!(u.new_startup, [["powershell", "-File", "scripts/daemon.ps1"]]);
    }

    #[test]
    fn pinned_refs_are_not_candidates() {
        let list = parse_list(LIST).unwrap();
        let ids: Vec<_> = candidates(&list).iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["herdr-file-viewer", "branch-tool"]);
    }

    #[test]
    fn ls_remote_picks_the_ref() {
        let out = "aaaa\tHEAD\nbbbb\trefs/heads/main\ncccc\trefs/heads/dev\ndddd\trefs/tags/v1\n";
        assert_eq!(parse_ls_remote(out, None).as_deref(), Some("aaaa"));
        assert_eq!(parse_ls_remote(out, Some("dev")).as_deref(), Some("cccc"));
        assert_eq!(parse_ls_remote(out, Some("refs/heads/main")).as_deref(), Some("bbbb"));
        // A tag does not move: it counts as pinned.
        assert_eq!(parse_ls_remote(out, Some("v1")), None);
        assert_eq!(parse_ls_remote(out, Some("nope")), None);
    }

    #[test]
    fn manifest_url_has_commit_and_subdir() {
        let list = parse_list(LIST).unwrap();
        let tool = list.iter().find(|p| p.id == "branch-tool").unwrap();
        assert_eq!(
            manifest_url(tool, "abc").unwrap(),
            "https://raw.githubusercontent.com/someone/tools/abc/plugins/branch-tool/herdr-plugin.toml"
        );
        assert_eq!(
            manifest_url(&viewer(), "abc").unwrap(),
            "https://raw.githubusercontent.com/smarzban/herdr-file-viewer/abc/herdr-plugin.toml"
        );
    }
}
