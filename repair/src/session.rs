//! The paths herdr keeps in `session.json`, which go stale when a repo folder is renamed or moved.
//!
//! Everything here works on the parsed JSON and on plain strings, so it is tested without a herdr.

use serde_json::Value;

/// Windows "verbatim" prefix; herdr stores the repo paths of a worktree space with it.
const VERBATIM: &str = r"\\?\";

/// The keys that hold a folder: the repo, its checkout, and the working directories of a workspace and
/// its panes. `worktree_space.key` is `<repo root>\.git` and is not checked on its own: it is derived
/// from the root, and rewritten together with it.
const PATH_KEYS: [&str; 4] = ["repo_root", "checkout_path", "identity_cwd", "cwd"];

pub fn strip_verbatim(path: &str) -> &str {
    path.strip_prefix(VERBATIM).unwrap_or(path)
}

fn canonical(path: &str) -> String {
    strip_verbatim(path).replace('/', "\\").trim_end_matches('\\').to_string()
}

fn folded(path: &str) -> String {
    canonical(path).to_lowercase()
}

fn is_under(path: &str, root: &str) -> bool {
    let (path, root) = (folded(path), folded(root));
    path == root || path.starts_with(&format!("{root}\\"))
}

/// `path` with the folder `old` replaced by `new`, keeping the verbatim prefix and a trailing
/// separator as they were; `None` if `path` is not `old` or something inside it.
pub fn replace_root(path: &str, old: &str, new: &str) -> Option<String> {
    if !is_under(path, old) {
        return None;
    }
    let prefix = if path.starts_with(VERBATIM) { VERBATIM } else { "" };
    let body = strip_verbatim(path).replace('/', "\\");
    let rest: String = body.chars().skip(canonical(old).chars().count()).collect();
    Some(format!("{prefix}{}{rest}", canonical(new)))
}

/// A stored path that no longer exists.
#[derive(Debug, PartialEq)]
pub struct Missing {
    /// Which workspace holds it (its label, or its id when it has none).
    pub workspace: String,
    pub key: String,
    pub path: String,
}

/// A repo whose folder is gone, with what is needed to look for where it went.
#[derive(Debug, PartialEq)]
pub struct BrokenRepo {
    /// The old repo folder, without the verbatim prefix.
    pub root: String,
    /// Checkouts of the same repo that still exist (linked worktrees): they keep a link to the repo's
    /// remote even though the repo folder is gone.
    pub checkouts: Vec<String>,
    /// A repo of a worktree space, as opposed to a plain folder a pane or workspace was opened in.
    pub repo: bool,
}

/// The highest folder of `path` that does not exist: the one that was renamed or moved (what is under
/// it is gone with it). `None` if all of it exists or it is a network path.
fn first_missing(path: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let path = canonical(path);
    if path.starts_with("\\\\") {
        return None;
    }
    let parts: Vec<&str> = path.split('\\').collect();
    (1..=parts.len()).find_map(|n| {
        let prefix = if n == 1 { format!("{}\\", parts[0]) } else { parts[..n].join("\\") };
        (!exists(&prefix)).then(|| prefix.trim_end_matches('\\').to_string())
    })
}

fn workspaces(doc: &Value) -> impl Iterator<Item = &Value> {
    doc["workspaces"].as_array().into_iter().flatten()
}

fn name(workspace: &Value) -> String {
    let label = workspace["worktree_space"]["label"].as_str().or(workspace["custom_name"].as_str());
    label.or(workspace["id"].as_str()).unwrap_or("?").to_string()
}

fn collect(value: &Value, workspace: &str, exists: &dyn Fn(&str) -> bool, out: &mut Vec<Missing>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                match v.as_str() {
                    Some(path) if PATH_KEYS.contains(&key.as_str()) => {
                        let seen = out.iter().any(|m| m.path == path && m.key == *key);
                        if !seen && !exists(strip_verbatim(path)) {
                            out.push(Missing { workspace: workspace.into(), key: key.clone(), path: path.into() });
                        }
                    }
                    _ => collect(v, workspace, exists, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect(v, workspace, exists, out)),
        _ => {}
    }
}

/// Every stored folder that does not exist on disk.
pub fn missing_paths(doc: &Value, exists: &dyn Fn(&str) -> bool) -> Vec<Missing> {
    let mut out = Vec::new();
    for workspace in workspaces(doc) {
        collect(workspace, &name(workspace), exists, &mut out);
    }
    out
}

/// The repos (one per distinct folder) whose root is gone.
pub fn broken_repos(doc: &Value, exists: &dyn Fn(&str) -> bool) -> Vec<BrokenRepo> {
    let mut repos: Vec<BrokenRepo> = Vec::new();
    for workspace in workspaces(doc) {
        let Some(root) = workspace["worktree_space"]["repo_root"].as_str() else { continue };
        let root = strip_verbatim(root);
        if exists(root) || repos.iter().any(|r| folded(&r.root) == folded(root)) {
            continue;
        }
        repos.push(BrokenRepo { root: root.into(), checkouts: Vec::new(), repo: true });
    }
    for workspace in workspaces(doc) {
        let space = &workspace["worktree_space"];
        let Some(checkout) = space["checkout_path"].as_str().map(strip_verbatim) else { continue };
        let Some(root) = space["repo_root"].as_str() else { continue };
        if let Some(repo) = repos.iter_mut().find(|r| folded(&r.root) == folded(root))
            && !is_under(checkout, &repo.root)
            && exists(checkout)
        {
            repo.checkouts.push(checkout.into());
        }
    }
    // Folders of workspaces that are not repos with worktrees (a plain workspace or pane): the same
    // rename breaks them, but there is no repo to compare, so they only get a proposal by name.
    for missing in missing_paths(doc, exists) {
        let Some(root) = first_missing(strip_verbatim(&missing.path), exists) else { continue };
        if !repos.iter().any(|r| is_under(&root, &r.root) || is_under(&r.root, &root)) {
            repos.push(BrokenRepo { root, checkouts: Vec::new(), repo: false });
        }
    }
    repos
}

/// Rewrites every stored folder inside an old folder of `moves` (old, new) to its new place, and the
/// label of a worktree space that was just the folder's name. Returns how many values changed.
pub fn rewrite(doc: &mut Value, moves: &[(String, String)]) -> usize {
    let mut changed = 0;
    for workspace in doc["workspaces"].as_array_mut().into_iter().flatten() {
        let root = workspace["worktree_space"]["repo_root"].as_str().map(str::to_string);
        let renamed = root.and_then(|r| moves.iter().find(|(old, _)| is_under(&r, old)).cloned());
        if let Some((old, new)) = renamed {
            let label = &mut workspace["worktree_space"]["label"];
            if label.as_str() == Some(last_component(&old)) {
                *label = Value::String(last_component(&new).to_string());
                changed += 1;
            }
        }
        changed += rewrite_paths(workspace, moves);
    }
    changed
}

pub fn last_component(path: &str) -> &str {
    strip_verbatim(path).trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or("")
}

fn rewrite_paths(value: &mut Value, moves: &[(String, String)]) -> usize {
    let mut changed = 0;
    match value {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                let is_path = PATH_KEYS.contains(&key.as_str())
                    || (key == "key" && v.as_str().is_some_and(|s| s.ends_with(".git")));
                let new = v
                    .as_str()
                    .filter(|_| is_path)
                    .and_then(|s| moves.iter().find_map(|(old, new)| replace_root(s, old, new)));
                match new {
                    Some(new) => {
                        *v = Value::String(new);
                        changed += 1;
                    }
                    None => changed += rewrite_paths(v, moves),
                }
            }
        }
        Value::Array(items) => changed += items.iter_mut().map(|v| rewrite_paths(v, moves)).sum::<usize>(),
        _ => {}
    }
    changed
}

/// `text` with every case-insensitive occurrence of `needle` replaced (ASCII case folding keeps byte
/// positions valid).
fn replace_ignoring_case(text: &str, needle: &str, with: &str) -> String {
    let (lower, needle) = (text.to_ascii_lowercase(), needle.to_ascii_lowercase());
    let (mut out, mut from) = (String::new(), 0);
    while let Some(at) = lower[from..].find(&needle).filter(|_| !needle.is_empty()) {
        out.push_str(&text[from..from + at]);
        out.push_str(with);
        from += at + needle.len();
    }
    out.push_str(&text[from..]);
    out
}

/// The lines of the `session.json` text that hold an old value of `pairs` (old, new), with what each
/// would become: what to edit by hand. Values are matched as they are written in JSON (backslashes
/// doubled), so a path stored with forward slashes is not found.
pub fn changed_lines(text: &str, pairs: &[(String, String)]) -> Vec<(usize, String, String)> {
    let json = |s: &str| s.replace('\\', "\\\\");
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let new =
                pairs.iter().fold(line.to_string(), |l, (old, new)| replace_ignoring_case(&l, &json(old), &json(new)));
            (new != line).then(|| (i + 1, line.trim().to_string(), new.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OLD: &str = r"C:\Users\alice\projects\app-old";
    const NEW: &str = r"C:\Users\alice\projects\app";

    fn doc() -> Value {
        json!({"workspaces": [
            {"id": "w1", "identity_cwd": r"C:\Users\alice\projects\app-old\",
             "worktree_space": {"key": r"\\?\C:\Users\alice\projects\app-old\.git", "label": "app-old",
                "repo_root": r"\\?\C:\Users\alice\projects\app-old", "checkout_path": r"\\?\C:\Users\alice\projects\app-old",
                "is_linked_worktree": false},
             "tabs": [{"panes": {"1": {"cwd": r"C:\Users\alice\projects\app-old\src"}}}]},
            {"id": "w2", "identity_cwd": r"C:\Users\alice\wt\app\feature\",
             "worktree_space": {"key": r"\\?\C:\Users\alice\projects\app-old\.git", "label": "app-old",
                "repo_root": r"\\?\C:\Users\alice\projects\app-old", "checkout_path": r"C:\Users\alice\wt\app\feature",
                "is_linked_worktree": true},
             "tabs": []},
            {"id": "w3", "identity_cwd": r"C:\Users\alice\gone\", "tabs": []},
        ]})
    }

    fn only_wt(path: &str) -> bool {
        path.starts_with(r"C:\Users\alice\wt")
    }

    #[test]
    fn replace_root_keeps_prefix_and_trailing_separator() {
        assert_eq!(
            replace_root(r"\\?\C:\Users\alice\projects\app-old", OLD, NEW).unwrap(),
            r"\\?\C:\Users\alice\projects\app"
        );
        assert_eq!(
            replace_root(r"c:\users\alice\projects\APP-OLD\src\", OLD, NEW).unwrap(),
            r"C:\Users\alice\projects\app\src\"
        );
        assert_eq!(replace_root(r"C:\Users\alice\projects\app-old2", OLD, NEW), None);
        assert_eq!(replace_root(r"C:\other", OLD, NEW), None);
    }

    #[test]
    fn finds_every_missing_folder_once() {
        let missing = missing_paths(&doc(), &only_wt);
        let paths: Vec<_> = missing.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.contains(&r"\\?\C:\Users\alice\projects\app-old"));
        assert!(paths.contains(&r"C:\Users\alice\projects\app-old\src"));
        assert!(paths.contains(&r"C:\Users\alice\gone\"));
        assert!(!paths.iter().any(|p| p.contains("feature")));
        assert_eq!(missing.iter().filter(|m| m.workspace == "app-old" && m.key == "repo_root").count(), 1);
    }

    #[test]
    fn broken_repo_remembers_its_surviving_checkouts() {
        let parents = [r"C:\", r"C:\Users", r"C:\Users\alice", r"C:\Users\alice\projects"];
        let disk = |p: &str| only_wt(p) || parents.contains(&p);
        let repos = broken_repos(&doc(), &disk);
        assert_eq!(
            repos,
            vec![
                BrokenRepo { root: OLD.into(), checkouts: vec![r"C:\Users\alice\wt\app\feature".into()], repo: true },
                // The plain workspace whose folder is gone too: it is not a repo.
                BrokenRepo { root: r"C:\Users\alice\gone".into(), checkouts: vec![], repo: false },
            ]
        );
    }

    #[test]
    fn the_renamed_folder_is_the_highest_one_that_is_missing() {
        let only_projects = |p: &str| p == r"C:\" || p == r"C:\Users" || p == r"C:\Users\alice";
        assert_eq!(
            first_missing(r"C:\Users\alice\notes\deep\file", &only_projects).as_deref(),
            Some(r"C:\Users\alice\notes")
        );
        assert_eq!(first_missing(r"\\server\share\x", &only_projects), None);
        assert_eq!(first_missing(r"C:\Users\alice", &only_projects), None);
    }

    #[test]
    fn rewrite_changes_the_repo_and_leaves_the_rest() {
        let mut doc = doc();
        let changed = rewrite(&mut doc, &[(OLD.into(), NEW.into())]);
        let space = &doc["workspaces"][0]["worktree_space"];
        assert_eq!(space["key"], r"\\?\C:\Users\alice\projects\app\.git");
        assert_eq!(space["repo_root"], r"\\?\C:\Users\alice\projects\app");
        assert_eq!(space["label"], "app");
        assert_eq!(doc["workspaces"][0]["tabs"][0]["panes"]["1"]["cwd"], r"C:\Users\alice\projects\app\src");
        assert_eq!(doc["workspaces"][1]["worktree_space"]["checkout_path"], r"C:\Users\alice\wt\app\feature");
        assert_eq!(doc["workspaces"][1]["worktree_space"]["key"], r"\\?\C:\Users\alice\projects\app\.git");
        assert_eq!(doc["workspaces"][2]["identity_cwd"], r"C:\Users\alice\gone\");
        // w1: key, repo_root, checkout_path, label, identity_cwd, pane cwd; w2: key, repo_root, label.
        assert_eq!(changed, 9);
        assert!(broken_repos(&doc, &only_wt).iter().all(|r| r.root != OLD));
    }

    #[test]
    fn lines_to_edit_by_hand_show_old_and_new() {
        let text = r#"{
  "label": "app-old",
  "repo_root": "C:\\Users\\alice\\projects\\APP-OLD",
  "x": 1
}"#;
        let pairs = vec![
            (OLD.to_string(), NEW.to_string()),
            (r#""label": "app-old""#.to_string(), r#""label": "app""#.to_string()),
        ];
        let lines = changed_lines(text, &pairs);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], (2, r#""label": "app-old","#.to_string(), r#""label": "app","#.to_string()));
        assert_eq!(lines[1].0, 3);
        assert_eq!(lines[1].2, r#""repo_root": "C:\\Users\\alice\\projects\\app","#);
    }
}
