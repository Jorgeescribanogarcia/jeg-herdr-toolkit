//! File preview: Markdown with `glow`, everything else with `bat` (highlighted like VS Code). If
//! either is missing, the plain text is shown: the viewer is still useful.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::Command;

use ansi_to_tui::IntoText;
use ratatui::text::Text;

const MAX_SIZE: u64 = 5 * 1024 * 1024;
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif", "tif", "tiff"];

pub fn render(path: &Path, width: u16) -> Text<'static> {
    let Ok(meta) = fs::metadata(path) else {
        return Text::raw("The file cannot be read.");
    };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if IMAGES.contains(&ext.as_str()) {
        // A terminal that speaks the Kitty graphics protocol (WezTerm) could draw it.
        return Text::raw(format!("Image ({}). Windows Terminal cannot display it.", size(meta.len())));
    }
    if meta.len() > MAX_SIZE {
        return Text::raw(format!("{} file: too large to preview.", size(meta.len())));
    }
    if is_binary(path) {
        return Text::raw(format!("Binary file ({}).", size(meta.len())));
    }

    let width = width.max(20).to_string();
    let highlighted = if matches!(ext.as_str(), "md" | "markdown" | "mdx") {
        // glow does not colour its output unless it writes to a terminal; CLICOLOR_FORCE makes it.
        run(Command::new("glow").args(["-s", "dark", "-w", &width]).arg(path).env("CLICOLOR_FORCE", "1"))
    } else {
        run(Command::new("bat")
            .args(["--color=always", "--paging=never", "--style=numbers", "--wrap=character"])
            .args(["--theme=Visual Studio Dark+", "--terminal-width", &width])
            .arg(path))
    };
    match highlighted {
        Some(text) => text,
        None => Text::raw(fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()),
    }
}

/// `None` if the program is missing or fails; the caller falls back to plain text.
fn run(command: &mut Command) -> Option<Text<'static>> {
    let output = command.env("COLORTERM", "truecolor").output().ok().filter(|s| s.status.success())?;
    output.stdout.into_text().ok()
}

/// Same rule as git: a NUL byte in the first 8000 means binary.
fn is_binary(path: &Path) -> bool {
    let mut head = [0u8; 8000];
    let read = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    head[..read].contains(&0)
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b if b >= 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}
