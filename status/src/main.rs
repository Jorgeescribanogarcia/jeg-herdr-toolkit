//! Custom agent status icons in herdr's sidebar. Run by herdr at startup and on its state-change
//! events; the work lives in `common::publish`, which the settings panel also uses when icons change.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use common::Settings;

fn main() -> ExitCode {
    let herdr = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let dir = env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from).unwrap_or_default();
    let (settings, warning) = Settings::load(&dir);
    if let Some(warning) = warning {
        eprintln!("herdr-status: {warning}; using the default icons");
    }
    match common::publish::publish(&herdr, &settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("herdr-status: {e}");
            ExitCode::FAILURE
        }
    }
}
