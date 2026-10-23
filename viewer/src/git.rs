//! Git state of each file, read from `git status --porcelain=v1 -z` (the format meant for programs,
//! which does not change with the language or the git version).

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use ratatui::style::Color;

/// Ordered from least to most important: a folder shows the most important state it contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Ignored,
    Renamed,
    Untracked,
    Added,
    Modified,
    Deleted,
    Conflict,
}

impl Status {
    /// Letter and colour of the VS Code decoration (default dark theme).
    pub fn letter(self) -> Option<char> {
        match self {
            Status::Ignored => None,
            Status::Renamed => Some('R'),
            Status::Untracked => Some('U'),
            Status::Added => Some('A'),
            Status::Modified => Some('M'),
            Status::Deleted => Some('D'),
            Status::Conflict => Some('!'),
        }
    }

    pub fn color(self) -> Color {
        match self {
            Status::Ignored => Color::Rgb(0x8c, 0x8c, 0x8c),
            Status::Renamed | Status::Untracked => Color::Rgb(0x73, 0xc9, 0x91),
            Status::Added => Color::Rgb(0x81, 0xb8, 0x8b),
            Status::Modified => Color::Rgb(0xe2, 0xc0, 0x8d),
            Status::Deleted => Color::Rgb(0xc7, 0x4e, 0x39),
            Status::Conflict => Color::Rgb(0xe4, 0x67, 0x6b),
        }
    }
}

#[derive(Default)]
pub struct GitStatus {
    pub branch: Option<String>,
    files: HashMap<String, Status>,
    folders: HashMap<String, Status>,
    /// Ignored paths as git reports them; folders end in «/» and include everything inside.
    ignored: Vec<String>,
    /// Where the viewer's folder sits inside the repo («tools/», or empty at the root). git reports
    /// paths from the repo root and the viewer asks from its own folder.
    prefix: String,
}

impl GitStatus {
    /// `None` if `root` is not a repo or git is not installed: the viewer still works, without colours.
    pub fn read(root: &Path) -> Option<GitStatus> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["status", "--porcelain=v1", "-z", "--branch", "--ignored=matching", "-uall"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let mut status = GitStatus::parse(&String::from_utf8_lossy(&output.stdout));
        let prefix = Command::new("git").arg("-C").arg(root).args(["rev-parse", "--show-prefix"]).output().ok()?;
        status.prefix = String::from_utf8_lossy(&prefix.stdout).trim().to_string();
        Some(status)
    }

    /// Parses the output of `git status --porcelain=v1 -z --branch`.
    fn parse(text: &str) -> GitStatus {
        let mut status = GitStatus::default();
        let mut entries = text.split('\0').filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            if let Some(branch) = entry.strip_prefix("## ") {
                status.branch = Some(branch_name(branch));
                continue;
            }
            let (xy, path) = entry.split_at(3.min(entry.len()));
            let xy = xy.as_bytes();
            if xy.len() < 2 {
                continue;
            }
            // For renames and copies, the next entry is the source path.
            if matches!(xy[0], b'R' | b'C') {
                entries.next();
            }
            if &xy[..2] == b"!!" {
                status.ignored.push(path.to_string());
                continue;
            }
            let s = status_of(xy[0], xy[1]);
            status.files.insert(path.to_string(), s);
            let mut parent = path;
            while let Some(i) = parent.rfind('/') {
                parent = &parent[..i];
                let current = status.folders.entry(parent.to_string()).or_insert(s);
                *current = (*current).max(s);
            }
        }
        status
    }

    /// `relative`: path from the viewer's folder, with «/».
    pub fn of(&self, relative: &str, is_folder: bool) -> Option<Status> {
        let relative = format!("{}{relative}", self.prefix);
        let relative = relative.as_str();
        let own = if is_folder { self.folders.get(relative) } else { self.files.get(relative) };
        own.copied().or_else(|| {
            let ignored = self.ignored.iter().any(|i| match i.strip_suffix('/') {
                Some(folder) => relative == folder || relative.starts_with(i.as_str()),
                None => relative == i,
            });
            ignored.then_some(Status::Ignored)
        })
    }
}

fn status_of(x: u8, y: u8) -> Status {
    match (x, y) {
        (b'?', b'?') => return Status::Untracked,
        (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => return Status::Conflict,
        _ => {}
    }
    // The working-tree change wins over the index one, as in VS Code's explorer.
    match if y != b' ' { y } else { x } {
        b'D' => Status::Deleted,
        b'A' | b'C' => Status::Added,
        b'R' => Status::Renamed,
        _ => Status::Modified,
    }
}

/// «main...origin/main [ahead 1]» → «main»; «No commits yet on main» → «main».
fn branch_name(header: &str) -> String {
    let header = header.strip_prefix("No commits yet on ").unwrap_or(header);
    header.split("...").next().unwrap_or(header).split(' ').next().unwrap_or(header).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = concat!(
        "## feature\0",
        " M .gitignore\0",
        "?? tools/viewer/src/git.rs\0",
        "!! .cache/build/.gitignore\0",
        "!! .cache/build/out/\0",
    );

    #[test]
    fn folder_with_only_ignored_entries_has_no_mark() {
        let s = GitStatus::parse(OUTPUT);
        assert_eq!(s.of(".cache", true), None);
        assert_eq!(s.of(".cache/build/out", true), Some(Status::Ignored));
        assert_eq!(s.of(".cache/build/out/log.txt", false), Some(Status::Ignored));
    }

    #[test]
    fn viewer_opened_in_a_repo_subfolder() {
        // git reports paths from the repo root; the viewer asks from its own folder.
        let mut s = GitStatus::parse(OUTPUT);
        s.prefix = "tools/".into();
        assert_eq!(s.of("viewer/src/git.rs", false), Some(Status::Untracked));
        assert_eq!(s.of("viewer", true), Some(Status::Untracked));
        assert_eq!(s.of(".gitignore", false), None);
    }

    #[test]
    fn changes_propagate_to_their_folders() {
        let s = GitStatus::parse(OUTPUT);
        assert_eq!(s.branch.as_deref(), Some("feature"));
        assert_eq!(s.of(".gitignore", false), Some(Status::Modified));
        assert_eq!(s.of("tools/viewer", true), Some(Status::Untracked));
        assert_eq!(s.of("tools", true), Some(Status::Untracked));
    }
}
