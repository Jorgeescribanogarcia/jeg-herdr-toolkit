//! herdr's public plugin index (`https://assets.herdr.dev/plugins/index.json`, the one its website
//! shows): parsing, search, filters and order. No network here.

use common::settings::{Marketplace, Sort};
use serde::Deserialize;

use crate::installed::{self, Installed};

pub const URL: &str = "https://assets.herdr.dev/plugins/index.json";

/// One installable plugin: a repo with several manifests gives one per manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// `owner/repo`
    pub repo: String,
    /// The manifest's path inside the repo (`herdr-plugin.toml`, `links/herdr-plugin.toml`).
    pub path: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub min_herdr: Option<String>,
    /// The manifest's; if it has none, the repo's.
    pub description: String,
    /// Empty: the manifest does not restrict them.
    pub platforms: Vec<String>,
    pub stars: u64,
    /// When it entered herdr's index (RFC 3339).
    pub first_seen: String,
    pub pushed: String,
    pub head_commit: String,
}

impl Listing {
    /// What `herdr plugin install` takes: `owner/repo`, plus the manifest's folder if it is not at the root.
    pub fn install_source(&self) -> String {
        match self.path.rsplit_once('/') {
            Some((dir, _)) => format!("{}/{dir}", self.repo),
            None => self.repo.clone(),
        }
    }

    pub fn runs_on_windows(&self) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|p| p == "windows")
    }
}

#[derive(Deserialize)]
struct Index {
    plugins: Vec<serde_json::Value>,
}

/// Every field optional: the index is built by herdr from third-party repos, and one odd entry (a
/// `null` where a string should be) must not throw away the other two thousand.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repo {
    full_name: Option<String>,
    description: Option<String>,
    stars: Option<u64>,
    first_seen_at: Option<String>,
    pushed_at: Option<String>,
    head_commit: Option<String>,
    manifests: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    path: Option<String>,
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    min_herdr_version: Option<String>,
    description: Option<String>,
    platforms: Option<Vec<String>>,
}

/// One `Listing` per usable manifest. Entries without a repo name, or manifests without an id or path,
/// are left out: they cannot be installed, and there is no one to tell about them.
pub fn parse(json: &str) -> Result<Vec<Listing>, String> {
    let index: Index = serde_json::from_str(json).map_err(|e| format!("herdr's plugin index is malformed: {e}"))?;
    let mut all = Vec::new();
    for repo in index.plugins.into_iter().filter_map(|v| serde_json::from_value::<Repo>(v).ok()) {
        let Some(full_name) = repo.full_name else { continue };
        let manifests = repo.manifests.unwrap_or_default();
        for m in manifests.into_iter().filter_map(|v| serde_json::from_value::<Manifest>(v).ok()) {
            let (Some(id), Some(path)) = (m.id, m.path) else { continue };
            all.push(Listing {
                repo: full_name.clone(),
                path,
                name: m.name.unwrap_or_else(|| id.clone()),
                id,
                version: m.version.unwrap_or_default(),
                min_herdr: m.min_herdr_version,
                description: m.description.or_else(|| repo.description.clone()).unwrap_or_default(),
                platforms: m.platforms.unwrap_or_default(),
                stars: repo.stars.unwrap_or(0),
                first_seen: repo.first_seen_at.clone().unwrap_or_default(),
                pushed: repo.pushed_at.clone().unwrap_or_default(),
                head_commit: repo.head_commit.clone().unwrap_or_default(),
            });
        }
    }
    Ok(all)
}

/// Every word of `query` has to appear (any case) in the name, id, description or `owner/repo`.
pub fn filter<'a>(all: &'a [Listing], query: &str, installed: &[Installed], opts: &Marketplace) -> Vec<&'a Listing> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    all.iter()
        .filter(|l| !opts.hide_other_platforms || l.runs_on_windows())
        .filter(|l| !(opts.hide_installed && installed.iter().any(|p| installed::matches(p, l))))
        .filter(|l| {
            let haystack = format!("{} {} {} {}", l.name, l.id, l.description, l.repo).to_lowercase();
            words.iter().all(|w| haystack.contains(w.as_str()))
        })
        .collect()
}

/// Highest first; ties by name so the order does not jump between refreshes.
pub fn sort(list: &mut [&Listing], by: Sort) {
    list.sort_by(|a, b| {
        let order = match by {
            Sort::Newest => b.first_seen.cmp(&a.first_seen),
            Sort::Popular => b.stars.cmp(&a.stars),
            Sort::Active => b.pushed.cmp(&a.pushed),
        };
        order.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Entered the index after the explorer was last opened. Never on the first opening: everything would be.
/// Both are RFC 3339 in UTC, which sort as text.
pub fn is_new(l: &Listing, last_open: Option<&str>) -> bool {
    last_open.is_some_and(|last| l.first_seen.as_str() > last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::settings::{Marketplace, Sort};

    const INDEX: &str = include_str!("testdata/index.json");

    fn all() -> Vec<Listing> {
        parse(INDEX).unwrap()
    }

    fn ids<'a>(list: &[&'a Listing]) -> Vec<&'a str> {
        list.iter().map(|l| l.id.as_str()).collect()
    }

    fn show_everything() -> Marketplace {
        Marketplace { hide_other_platforms: false, hide_installed: false, ..Marketplace::default() }
    }

    #[test]
    fn one_row_per_manifest() {
        let all = all();
        let annotate: Vec<_> = all.iter().filter(|l| l.repo == "plannotator/herdr-annotate").collect();
        assert_eq!(annotate.len(), 2);
        assert_eq!(annotate[0].id, "annotate");
        assert_eq!(annotate[0].install_source(), "plannotator/herdr-annotate");
        assert_eq!(annotate[1].id, "annotate-links");
        assert_eq!(annotate[1].install_source(), "plannotator/herdr-annotate/links");
        assert_eq!(annotate[0].stars, 696);
        assert_eq!(annotate[0].version, "0.9.1");
        assert_eq!(annotate[0].min_herdr.as_deref(), Some("0.8.0"));
    }

    #[test]
    fn tolerates_missing_fields() {
        let all = all();
        // The empty repo has no manifests: no rows.
        assert!(all.iter().all(|l| l.repo != "someone/empty-repo"));
        assert_eq!(all.len(), 5);
        // Neither the manifest nor the repo has a description.
        let harbor = all.iter().find(|l| l.id == "herdr.harbor").unwrap();
        assert_eq!(harbor.description, "");
        // The manifest has none, the repo does: the repo's is used.
        let log = all.iter().find(|l| l.id == "waynewu411.herdr-event-log").unwrap();
        assert!(log.description.starts_with("Herdr plugin: logs"), "{}", log.description);
        // No `platforms`: it does not restrict, so it runs on Windows.
        assert!(log.platforms.is_empty());
        assert!(log.runs_on_windows());
        assert!(parse("{\"plugins\": 3}").is_err());
    }

    #[test]
    fn search_matches_name_description_or_author() {
        let all = all();
        let opts = show_everything();
        assert_eq!(ids(&filter(&all, "ANNOTATE LINKS", &[], &opts)), ["annotate-links"]);
        assert_eq!(ids(&filter(&all, "browser inside", &[], &opts)), ["zenbu-labs.terminal-browser"]);
        assert_eq!(ids(&filter(&all, "zlj-zz", &[], &opts)), ["herdr.harbor"]);
        assert_eq!(filter(&all, "", &[], &opts).len(), 5);
        // Every word must match, in any field.
        assert_eq!(ids(&filter(&all, "plannotator links", &[], &opts)), ["annotate-links"]);
    }

    #[test]
    fn hides_other_platforms_and_installed() {
        let all = all();
        let opts = Marketplace { hide_other_platforms: true, hide_installed: true, ..Marketplace::default() };
        let mut installed = crate::installed::parse_list(include_str!("testdata/plugin-list.json")).unwrap();
        installed[3].id = "annotate".into(); // the local one: matched by id
        assert_eq!(ids(&filter(&all, "", &installed, &opts)), ["annotate-links", "waynewu411.herdr-event-log"]);
    }

    #[test]
    fn one_bad_entry_does_not_hide_the_rest() {
        let json = r#"{"plugins": [
            {"fullName": "a/one", "stars": null, "pushedAt": null, "manifests": [
                {"path": "herdr-plugin.toml", "id": null, "name": "broken"},
                {"path": "x/herdr-plugin.toml", "id": "good", "name": "Good", "version": null}
            ]},
            {"fullName": null, "manifests": []},
            "not even an object",
            {"fullName": "b/two", "manifests": [{"path": "herdr-plugin.toml", "id": "two", "name": "Two"}]}
        ]}"#;
        let ids: Vec<String> = parse(json).unwrap().into_iter().map(|l| l.id).collect();
        assert_eq!(ids, ["good", "two"]);
    }

    #[test]
    fn fork_with_the_same_id_is_not_installed() {
        let all = all();
        let mut fork = crate::installed::parse_list(include_str!("testdata/plugin-list.json")).unwrap();
        fork.truncate(1);
        fork[0].id = "annotate".into(); // same id, but installed from smarzban/herdr-file-viewer
        let opts = Marketplace { hide_other_platforms: false, hide_installed: true, ..Marketplace::default() };
        assert!(ids(&filter(&all, "", &fork, &opts)).contains(&"annotate"));
        // Linked from a local folder there is no repo to compare: the id decides.
        fork[0].repo = None;
        assert!(!ids(&filter(&all, "", &fork, &opts)).contains(&"annotate"));
        // Same repo, different case: installed.
        fork[0].repo = Some("PlanNotator/herdr-annotate".into());
        assert!(!ids(&filter(&all, "", &fork, &opts)).contains(&"annotate"));
    }

    #[test]
    fn sort_orders() {
        let all = all();
        let mut list = filter(&all, "", &[], &show_everything());
        sort(&mut list, Sort::Popular);
        assert_eq!(list[0].id, "zenbu-labs.terminal-browser"); // 4178 stars
        sort(&mut list, Sort::Newest);
        assert_eq!(list[0].id, "herdr.harbor"); // first seen 2026-09-05
        sort(&mut list, Sort::Active);
        assert_eq!(list[0].id, "zenbu-labs.terminal-browser"); // pushed 2026-10-21
    }

    #[test]
    fn new_since_last_open() {
        let all = all();
        let harbor = all.iter().find(|l| l.id == "herdr.harbor").unwrap();
        assert!(!is_new(harbor, None));
        assert!(is_new(harbor, Some("2026-09-01T00:00:00Z")));
        assert!(!is_new(harbor, Some("2026-09-06T00:00:00Z")));
    }
}
