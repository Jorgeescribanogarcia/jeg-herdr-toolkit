//! Writing files without ever leaving one half-written.

use std::fs;
use std::io;
use std::path::Path;

/// Writes to a temporary file next to the target and renames it over it. A concurrent reader (the
/// status icons run on every herdr event) sees either the old or the new content, never an empty
/// file; and if the process dies halfway, the real file is untouched. On Windows, `fs::rename`
/// replaces the destination if it exists.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| io::Error::other("path has no file name"))?;
    let temporary = path.with_file_name(format!("{}.new", name.to_string_lossy()));
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        // The temporary is useless if it could not be moved into place; the rename error is what matters.
        let _ = fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_and_leaves_no_temporaries() {
        let dir = std::env::temp_dir().join(format!("herdr-common-fs-util-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.toml");
        fs::write(&path, "old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }
}
