//! The file tree, flattened into its visible rows. It is re-read from disk when a folder is opened or
//! closed and on every refresh, so it always shows what is there (a new file appears on its own).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// What VS Code hides by default (`files.exclude`).
const HIDDEN: &[&str] = &[".git", ".svn", ".hg", "CVS", ".DS_Store", "Thumbs.db"];

pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub is_folder: bool,
    pub depth: usize,
}

/// `(path, is_folder) -> hide`. The tree knows nothing about git: its user decides what to leave out.
pub type Filter = Box<dyn Fn(&Path, bool) -> bool>;

pub struct Tree {
    pub root: PathBuf,
    open: HashSet<PathBuf>,
    pub rows: Vec<Row>,
    /// What it hides is neither shown nor walked into. Applied on the next `reload`.
    pub filter: Option<Filter>,
}

impl Tree {
    pub fn new(root: PathBuf) -> Tree {
        let mut tree = Tree { root, open: HashSet::new(), rows: Vec::new(), filter: None };
        tree.reload();
        tree
    }

    pub fn reload(&mut self) {
        self.rows.clear();
        let root = self.root.clone();
        self.add_children(&root, 0);
    }

    fn add_children(&mut self, folder: &Path, depth: usize) {
        // A folder without read permission stays empty instead of bringing the viewer down.
        let Ok(entries) = fs::read_dir(folder) else { return };
        let mut children: Vec<(String, PathBuf, bool)> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if HIDDEN.contains(&name.as_str()) {
                    return None;
                }
                let path = e.path();
                let is_folder = path.is_dir();
                if self.filter.as_ref().is_some_and(|hide| hide(&path, is_folder)) {
                    return None;
                }
                Some((name, path, is_folder))
            })
            .collect();
        // Like VS Code's explorer: folders first, then by name, case-insensitively.
        children.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        for (name, path, is_folder) in children {
            let open = is_folder && self.open.contains(&path);
            self.rows.push(Row { path: path.clone(), name, is_folder, depth });
            if open {
                self.add_children(&path, depth + 1);
            }
        }
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.open.contains(path)
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.open.remove(path) {
            self.open.insert(path.to_path_buf());
        }
        self.reload();
    }

    pub fn close(&mut self, path: &Path) {
        if self.open.remove(path) {
            self.reload();
        }
    }

    /// Index of the folder containing row `i`, unless it is at the top level.
    pub fn parent(&self, i: usize) -> Option<usize> {
        let depth = self.rows.get(i)?.depth;
        (depth > 0).then(|| (0..i).rev().find(|&j| self.rows[j].depth < depth)).flatten()
    }

    pub fn position(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|r| r.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_hides_rows_and_does_not_walk_into_them() {
        let dir = std::env::temp_dir().join(format!("herdr-viewer-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("target/release")).unwrap();
        fs::write(dir.join("a.txt"), "").unwrap();
        fs::write(dir.join("b.log"), "").unwrap();
        let mut tree = Tree::new(dir.clone());
        tree.filter =
            Some(Box::new(|path: &Path, _| path.extension().is_some_and(|e| e == "log") || path.ends_with("target")));
        tree.reload();
        let names: Vec<&str> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a.txt"]);
        tree.filter = None;
        tree.reload();
        assert_eq!(tree.rows.len(), 3);
    }
}
