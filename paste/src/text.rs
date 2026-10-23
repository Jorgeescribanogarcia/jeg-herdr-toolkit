//! What gets typed into the pane, and housekeeping of the saved images. No clipboard access here,
//! so it can be tested.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Paths separated by spaces, each in double quotes if it has spaces or other characters a shell
/// or an agent would split on. A trailing space lets you keep typing right after.
pub fn paths_to_type(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        let path = path.display().to_string();
        let needs_quotes = path.chars().any(|c| c.is_whitespace() || "&()[]{}^=;!'+,`~$#@%".contains(c));
        if needs_quotes {
            out.push_str(&format!("\"{path}\" "));
        } else {
            out.push_str(&path);
            out.push(' ');
        }
    }
    out
}

/// `clipboard-<unix ms>.png`: unique per paste and sorted by time.
pub fn image_name(now: SystemTime) -> String {
    let ms = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    format!("clipboard-{ms}.png")
}

/// Removes the plugin's saved images older than `max_age`. Only touches `clipboard-*.png`.
pub fn remove_old_images(dir: &Path, max_age: Duration, now: SystemTime) -> io::Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(dir)?.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("clipboard-") && name.ends_with(".png")) {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified());
        let old = modified.is_ok_and(|m| now.duration_since(m).is_ok_and(|age| age > max_age));
        if old && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn plain_paths_go_as_they_are() {
        let paths = [PathBuf::from(r"C:\tmp\a.png"), PathBuf::from(r"D:\b.txt")];
        assert_eq!(paths_to_type(&paths), r"C:\tmp\a.png D:\b.txt ");
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        let paths = [PathBuf::from(r"C:\My Files\shot 1.png")];
        assert_eq!(paths_to_type(&paths), r#""C:\My Files\shot 1.png" "#);
    }

    #[test]
    fn image_name_uses_milliseconds() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_792_700_000_123);
        assert_eq!(image_name(t), "clipboard-1792700000123.png");
    }

    #[test]
    fn only_old_clipboard_images_are_removed() {
        let dir = std::env::temp_dir().join(format!("herdr-paste-clean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let old = now - Duration::from_secs(8 * 24 * 3600);
        for (name, modified) in [("clipboard-1.png", old), ("clipboard-2.png", now), ("mine.png", old)] {
            File::create(dir.join(name)).unwrap().set_modified(modified).unwrap();
        }
        assert_eq!(remove_old_images(&dir, Duration::from_secs(7 * 24 * 3600), now).unwrap(), 1);
        let mut left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        left.sort();
        assert_eq!(left, ["clipboard-2.png", "mine.png"]);
    }
}
