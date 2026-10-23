//! Text that comes from third parties (herdr's index, other plugins' manifests): cleaned before it is
//! drawn, and checked before it reaches a command line.

/// Without control characters: an escape sequence in a description would otherwise reach the terminal.
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// `owner/repo` as GitHub allows it. What reaches `herdr plugin install` or a URL is checked here first,
/// so a name from the index can never be taken as an option (`-x`) or climb out of a path (`..`).
pub fn valid_repo(s: &str) -> bool {
    let parts: Vec<&str> = s.split('/').collect();
    parts.len() == 2 && parts.iter().all(|p| valid_segment(p))
}

/// A manifest's folder inside its repo (`links`, `plugins/one`).
pub fn valid_subdir(s: &str) -> bool {
    s.split('/').all(valid_segment)
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_drops_control_characters() {
        assert_eq!(clean("\x1b[31mred\x07 text\r\n"), "[31mred text");
        assert_eq!(clean("tab\there"), "tabhere");
        assert_eq!(clean("naïve · 󰁪"), "naïve · 󰁪");
    }

    #[test]
    fn valid_repo_names() {
        assert!(valid_repo("a-b/c.d"));
        assert!(valid_repo("Owner_1/repo-2"));
        for bad in ["-a/b", "a/-b", "a/b/c", "a b/c", "../x", "a/..", "a/.", "", "a/", "/b", "a"] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    #[test]
    fn valid_subdir_paths() {
        assert!(valid_subdir("links"));
        assert!(valid_subdir("plugins/one"));
        for bad in ["..", "a/../b", "/abs", "-x", r"a\b", "a//b", ""] {
            assert!(!valid_subdir(bad), "{bad}");
        }
    }
}
