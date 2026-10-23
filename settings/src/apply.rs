//! Saving a change from the panel: `settings.toml` and the plugin's keys in `config.toml`, reloading
//! herdr and, if herdr does not accept it, leaving both files as they were.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use common::Settings;
use common::fs_util::write_atomic;
use common::settings::FILE;
use serde_json::Value;

use crate::herdr_config;

/// What `save_and_reload` needs, kept apart so it can be tested without a real herdr.
pub struct Env<'a> {
    pub settings_dir: PathBuf,
    /// herdr's `config.toml`.
    pub config: PathBuf,
    pub plugin: String,
    /// Runs `herdr <args>` and returns its JSON reply.
    pub herdr: &'a dyn Fn(&[&str]) -> Result<Value, String>,
    /// Publishes the status icons again with the new settings.
    pub publish: &'a dyn Fn(&Settings) -> Result<(), String>,
}

pub fn save_and_reload(env: &Env, settings: &Settings) -> Result<(), String> {
    settings.validate()?;
    let settings_path = env.settings_dir.join(FILE);
    // A missing file is normal; one that exists and cannot be read is not: stop without writing,
    // because carrying on would overwrite it with a version that does not include it.
    let config_before = read_if_exists(&env.config)?;
    let settings_before = read_if_exists(&settings_path)?;
    let new_config = herdr_config::apply(config_before.as_deref().unwrap_or_default(), settings, &env.plugin)?;

    // A copy on disk while the change lasts: if the process dies before finishing, it stays there.
    let backup = env.config.with_extension("toml.bak");
    if let Some(text) = &config_before {
        write_atomic(&backup, text.as_bytes()).map_err(|e| format!("could not back up config.toml: {e}"))?;
    }

    let result = (|| {
        settings.save(&env.settings_dir).map_err(|e| format!("could not save settings.toml: {e}"))?;
        write_atomic(&env.config, new_config.as_bytes()).map_err(|e| format!("could not write config.toml: {e}"))?;
        let reply = (env.herdr)(&["server", "reload-config"])?;
        let result = &reply["result"];
        let diagnostics = result["diagnostics"].as_array().cloned().unwrap_or_default();
        if result["status"].as_str() != Some("applied") || !diagnostics.is_empty() {
            return Err(format!("herdr did not accept the configuration: {}", Value::Array(diagnostics)));
        }
        Ok(())
    })();

    if let Err(error) = result {
        let undone = restore(&env.config, config_before.as_deref())
            .and_then(|()| restore(&settings_path, settings_before.as_deref()));
        // Load the previous configuration again; if this also fails, herdr keeps the one it had in
        // memory, which is the good one: the first error is the one that matters.
        let _ = (env.herdr)(&["server", "reload-config"]);
        return Err(match undone {
            Ok(()) => {
                let _ = fs::remove_file(&backup); // already restored: the backup is not needed
                error
            }
            Err(e) => format!("{error}. And the files could not be restored!: {e}. Backup at {}", backup.display()),
        });
    }
    // Done: the backup is not needed. If it cannot be removed it is a nuisance, not a problem.
    let _ = fs::remove_file(&backup);
    (env.publish)(settings)
}

fn read_if_exists(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {} ({e}); nothing was changed", path.display())),
    }
}

/// Leaves the file as it was: with its content, or removed if it did not exist.
fn restore(path: &Path, before: Option<&str>) -> io::Result<()> {
    match before {
        Some(text) => write_atomic(path, text.as_bytes()),
        None => fs::remove_file(path).or_else(|e| if e.kind() == io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    const CONFIG: &str = "# mine\n[ui]\nstatus_indicators = \"symbols\"\n";

    fn prepare(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("herdr-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.toml");
        fs::write(&config, CONFIG).unwrap();
        Settings::default().save(&dir).unwrap();
        (dir, config)
    }

    fn changed() -> Settings {
        let mut s = Settings::default();
        s.icons.set = "text".into();
        s
    }

    #[test]
    fn herdr_rejects_it_and_everything_is_rolled_back() {
        let (dir, config) = prepare("rejected");
        let settings_before = fs::read_to_string(dir.join(FILE)).unwrap();
        let published = Cell::new(false);
        let herdr = |_: &[&str]| Ok(json!({"result": {"status": "applied", "diagnostics": [{"message": "bad"}]}}));
        let publish = |_: &Settings| {
            published.set(true);
            Ok(())
        };
        let env = Env {
            settings_dir: dir.clone(),
            config: config.clone(),
            plugin: "p".into(),
            herdr: &herdr,
            publish: &publish,
        };
        let error = save_and_reload(&env, &changed()).unwrap_err();
        assert!(error.contains("bad"), "{error}");
        assert_eq!(fs::read_to_string(&config).unwrap(), CONFIG);
        assert_eq!(fs::read_to_string(dir.join(FILE)).unwrap(), settings_before);
        assert!(!published.get());
    }

    #[test]
    fn herdr_accepts_it_and_it_is_published() {
        let (dir, config) = prepare("accepted");
        let calls = Cell::new(0);
        let herdr = |args: &[&str]| {
            assert_eq!(args, ["server", "reload-config"]);
            calls.set(calls.get() + 1);
            Ok(json!({"result": {"status": "applied", "diagnostics": []}}))
        };
        let publish = |s: &Settings| {
            assert_eq!(s.icons.set, "text");
            Ok(())
        };
        let env = Env {
            settings_dir: dir.clone(),
            config: config.clone(),
            plugin: "p".into(),
            herdr: &herdr,
            publish: &publish,
        };
        save_and_reload(&env, &changed()).unwrap();
        assert_eq!(calls.get(), 1);
        assert!(fs::read_to_string(&config).unwrap().starts_with("# mine\n"));
        assert!(fs::read_to_string(&config).unwrap().contains("starts_with = \"✔\""));
        assert_eq!(Settings::load(&dir).0.icons.set, "text");
    }

    #[test]
    fn unreadable_config_is_not_touched() {
        // A Latin-1 comment makes the file invalid UTF-8: reading it fails although it exists.
        let (dir, config) = prepare("unreadable");
        let bytes = b"# a\xf1o\n[ui]\nstatus_indicators = \"symbols\"\n".to_vec();
        fs::write(&config, &bytes).unwrap();
        let herdr = |_: &[&str]| -> Result<Value, String> { panic!("must not reload") };
        let publish = |_: &Settings| Ok(());
        let env = Env {
            settings_dir: dir.clone(),
            config: config.clone(),
            plugin: "p".into(),
            herdr: &herdr,
            publish: &publish,
        };
        assert!(save_and_reload(&env, &changed()).is_err());
        assert_eq!(fs::read(&config).unwrap(), bytes);
        assert_eq!(Settings::load(&dir).0.icons.set, "material");
    }

    #[test]
    fn backup_exists_on_disk_during_reload_and_nothing_is_left_after() {
        let (dir, config) = prepare("backup");
        let backup = config.with_extension("toml.bak");
        let herdr = |_: &[&str]| {
            assert_eq!(fs::read_to_string(&backup).unwrap(), CONFIG);
            Ok(json!({"result": {"status": "applied", "diagnostics": []}}))
        };
        let publish = |_: &Settings| Ok(());
        let env = Env {
            settings_dir: dir.clone(),
            config: config.clone(),
            plugin: "p".into(),
            herdr: &herdr,
            publish: &publish,
        };
        save_and_reload(&env, &changed()).unwrap();
        assert!(!backup.exists());
        let left: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 2, "{left:?}"); // config.toml and settings.toml
    }

    #[test]
    fn invalid_settings_touch_nothing() {
        let (dir, config) = prepare("invalid");
        let herdr = |_: &[&str]| -> Result<Value, String> { panic!("must not reload") };
        let publish = |_: &Settings| Ok(());
        let env = Env {
            settings_dir: dir.clone(),
            config: config.clone(),
            plugin: "p".into(),
            herdr: &herdr,
            publish: &publish,
        };
        let mut bad = Settings::default();
        bad.viewer.refresh_secs = 0;
        assert!(save_and_reload(&env, &bad).is_err());
        assert_eq!(fs::read_to_string(&config).unwrap(), CONFIG);
    }
}
