//! The plugins herdr has installed, from `herdr plugin list --json`.

use serde::Deserialize;

/// A plugin as herdr reports it. Commands are only the ones that run on Windows: a manifest can limit
/// each `[[build]]` or `[[startup]]` entry to some platforms, and herdr skips the others.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `owner/repo`; `None` for a plugin linked from a local folder.
    pub repo: Option<String>,
    /// The manifest's folder inside the repo, if it is not at the root.
    pub subdir: Option<String>,
    /// The `--ref` it was installed with; `None` follows the repo's default branch.
    pub git_ref: Option<String>,
    pub commit: Option<String>,
    pub build: Vec<Vec<String>>,
    pub startup: Vec<Vec<String>>,
}

impl Installed {
    /// Installed at a fixed commit: `herdr plugin update` never moves it, so it is not offered.
    pub fn pinned(&self) -> bool {
        self.git_ref.as_deref().is_some_and(|r| r.len() == 40 && r.chars().all(|c| c.is_ascii_hexdigit()))
    }
}

#[derive(Deserialize)]
struct List {
    result: ListResult,
}

#[derive(Deserialize)]
struct ListResult {
    plugins: Vec<Plugin>,
}

#[derive(Deserialize)]
struct Plugin {
    plugin_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    build: Vec<Command>,
    #[serde(default)]
    startup: Vec<Command>,
    source: Source,
}

#[derive(Deserialize)]
struct Source {
    kind: String,
    owner: Option<String>,
    repo: Option<String>,
    subdir: Option<String>,
    requested_ref: Option<String>,
    resolved_commit: Option<String>,
}

#[derive(Deserialize)]
struct Command {
    command: Vec<String>,
    platforms: Option<Vec<String>>,
}

fn for_windows(commands: Vec<Command>) -> Vec<Vec<String>> {
    commands
        .into_iter()
        .filter(|c| c.platforms.as_ref().is_none_or(|p| p.iter().any(|p| p == "windows")))
        .map(|c| c.command)
        .collect()
}

pub fn parse_list(json: &str) -> Result<Vec<Installed>, String> {
    let list: List =
        serde_json::from_str(json).map_err(|e| format!("herdr plugin list is not what was expected: {e}"))?;
    Ok(list
        .result
        .plugins
        .into_iter()
        .map(|p| {
            let repo = match (p.source.kind.as_str(), p.source.owner, p.source.repo) {
                ("github", Some(owner), Some(repo)) => Some(format!("{owner}/{repo}")),
                _ => None,
            };
            Installed {
                id: p.plugin_id,
                name: p.name,
                version: p.version,
                repo,
                subdir: p.source.subdir.filter(|s| !s.is_empty()),
                git_ref: p.source.requested_ref.filter(|r| !r.is_empty()),
                commit: p.source.resolved_commit,
                build: for_windows(p.build),
                startup: for_windows(p.startup),
            }
        })
        .collect())
}

/// What `herdr plugin list --json` reports now.
pub fn list(herdr: &str) -> Result<Vec<Installed>, String> {
    let output = std::process::Command::new(herdr)
        .args(["plugin", "list", "--json"])
        .output()
        .map_err(|e| format!("could not run herdr: {e}"))?;
    if !output.status.success() {
        return Err(format!("herdr plugin list: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_list(&String::from_utf8_lossy(&output.stdout))
}

pub fn installed_ids(list: &[Installed]) -> Vec<String> {
    list.iter().map(|p| p.id.clone()).collect()
}

/// Whether `l` is this installed plugin. Forks in the index reuse their original's id, so the repo has
/// to match too; a plugin linked from a local folder has no repo, and its id decides.
pub fn matches(p: &Installed, l: &crate::index::Listing) -> bool {
    p.id == l.id && p.repo.as_deref().is_none_or(|r| r.eq_ignore_ascii_case(&l.repo))
}

/// The parts of a `herdr-plugin.toml` the marketplace shows: its version and what it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub build: Vec<Vec<String>>,
    pub startup: Vec<Vec<String>>,
}

#[derive(Deserialize)]
struct ManifestFile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    build: Vec<Command>,
    #[serde(default)]
    startup: Vec<Command>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: ManifestFile = toml::from_str(text).map_err(|e| format!("the plugin's manifest is malformed: {e}"))?;
        Ok(Manifest { version: m.version, build: for_windows(m.build), startup: for_windows(m.startup) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = include_str!("testdata/plugin-list.json");

    fn find<'a>(list: &'a [Installed], id: &str) -> &'a Installed {
        list.iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn local_has_no_repo() {
        let list = parse_list(LIST).unwrap();
        let local = find(&list, "jeg-herdr-toolkit");
        assert_eq!((local.repo.as_deref(), local.commit.as_deref()), (None, None));
    }

    #[test]
    fn github_fields() {
        let list = parse_list(LIST).unwrap();
        let viewer = find(&list, "herdr-file-viewer");
        assert_eq!(viewer.repo.as_deref(), Some("smarzban/herdr-file-viewer"));
        assert_eq!(viewer.git_ref, None);
        assert_eq!(viewer.commit.as_deref(), Some("28f11cce5b9ed02cabe8b5c61756e2ed3b7ac4ff"));
        assert_eq!(viewer.version, "1.18.0");
        // Only what runs on Windows.
        assert_eq!(viewer.build.len(), 1);
        assert_eq!(viewer.build[0].last().unwrap(), "scripts/fetch-or-build.ps1");
        let tool = find(&list, "branch-tool");
        assert_eq!(tool.subdir.as_deref(), Some("plugins/branch-tool"));
        assert_eq!(tool.git_ref.as_deref(), Some("dev"));
        assert_eq!(tool.startup, [["target/release/tool.exe", "daemon"]]);
        assert!(find(&list, "pinned-viewer").pinned());
        assert!(!tool.pinned());
        assert_eq!(installed_ids(&list).len(), 4);
    }

    #[test]
    fn manifest_commands_for_windows() {
        let manifest = include_str!("testdata/manifest-new.toml");
        let m = Manifest::parse(manifest).unwrap();
        assert_eq!(m.version, "1.19.0");
        assert_eq!(
            m.build,
            [["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/install.ps1"]]
        );
        assert_eq!(m.startup, [["powershell", "-File", "scripts/daemon.ps1"]]);
        assert!(Manifest::parse("version = [").is_err());
    }
}
