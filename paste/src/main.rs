//! Pastes the clipboard into the active pane as paths: files copied in Explorer become their paths;
//! an image (a screenshot, an image copied from a browser) is saved as a PNG and its path is typed.
//! Agents such as Claude Code read the file behind a path.
//!
//! herdr's own clipboard-image bridge only works with `herdr --remote`; on local Windows panes it is
//! not wired, which is what this action covers.

mod text;

use std::env;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, SystemTime};

use arboard::{Clipboard, ImageData};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Saved images older than this are removed on the next paste.
const KEEP_IMAGES: Duration = Duration::from_secs(7 * 24 * 3600);

fn main() -> ExitCode {
    let herdr = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    match paste(&herdr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("herdr-paste: {e}");
            notify(&herdr, "Nothing pasted", &e.to_string());
            ExitCode::FAILURE
        }
    }
}

fn paste(herdr: &str) -> AppResult<()> {
    let pane = env::var("HERDR_PANE_ID").map_err(|_| "herdr did not pass the active pane (HERDR_PANE_ID)")?;
    let mut clipboard = Clipboard::new()?;

    // Files first: copying a file in Explorer can also put its icon on the clipboard as an image.
    let to_type = match clipboard.get().file_list() {
        Ok(files) if !files.is_empty() => text::paths_to_type(&files),
        _ => match clipboard.get_image() {
            Ok(image) => text::paths_to_type(&[save_image(&image)?]),
            Err(_) => return Err("the clipboard has no files or image".into()),
        },
    };

    let output = Command::new(herdr).args(["pane", "send-text", &pane, &to_type]).output()?;
    if !output.status.success() {
        return Err(format!("herdr did not accept the text: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

/// Saves the image in the plugin's state folder (herdr creates it; the temp folder if run by hand).
fn save_image(image: &ImageData) -> AppResult<PathBuf> {
    let dir = env::var_os("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from).unwrap_or_else(env::temp_dir).join("clipboard");
    fs::create_dir_all(&dir)?;
    let now = SystemTime::now();
    // Housekeeping must not stop the paste: at worst old images stay until the next time.
    let _ = text::remove_old_images(&dir, KEEP_IMAGES, now);
    let path = dir.join(text::image_name(now));
    write_png(&path, image)?;
    Ok(path)
}

fn write_png(path: &Path, image: &ImageData) -> AppResult<()> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), image.width as u32, image.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.bytes)?;
    Ok(())
}

/// A herdr notification, so a failed paste does not pass silently (the action has no terminal).
fn notify(herdr: &str, title: &str, body: &str) {
    // If even the notification fails there is nowhere left to report it; the plugin log has the error.
    let _ = Command::new(herdr).args(["notification", "show", title, "--body", body]).output();
}
