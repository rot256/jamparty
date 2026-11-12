//! The joiner's settings file: `settings.json` next to the key
//! (`~/.config/jampot/settings.json`), mode 0600 in a 0700 directory.
//! Each session is its own browser origin, so the page cannot keep them;
//! the joiner process does. A file open to other users is refused, as the
//! key file is.

use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use nix::fcntl::{Flock, FlockArg};
use rootcause::prelude::{ResultExt, bail, report};
use uuid::Uuid;

use crate::{
    Report,
    adapters::keyfile,
    core::settings::{Settings, SettingsChange},
};

const FILE: &str = "settings.json";
const LOCK: &str = "settings.lock";
/// Settings files are small; a larger one is not ours.
const MAX_BYTES: u64 = 64 * 1024;

/// Where the settings live, from the environment.
pub(crate) fn default_path() -> Result<PathBuf, Report> {
    Ok(keyfile::config_dir()?.join(FILE))
}

/// The settings at `path`; the defaults when there is no file yet.
pub(crate) fn load(path: &Path) -> Result<Settings, Report> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Settings::default()),
        Err(error) => {
            return Err(report!(error)
                .context(format!("could not read {}", path.display()))
                .into());
        }
    };
    let mode = metadata.mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} is open to other users (mode {:o}); chmod 600 it, or delete it",
            path.display(),
            mode & 0o777
        );
    }
    if metadata.len() > MAX_BYTES {
        bail!("{} is larger than {MAX_BYTES} bytes", path.display());
    }
    let text =
        fs::read_to_string(path).context_with(|| format!("could not read {}", path.display()))?;
    Ok(serde_json::from_str(&text)
        .context_with(|| format!("{} is not valid; fix or delete it", path.display()))?)
}

/// Applies `change` to the settings at `path` as they are on disk now and
/// writes them back; returns the result. Several joiner processes may share
/// a home directory: an exclusive lock on `settings.lock` next to the file
/// makes each change read, apply and write without losing another's.
pub(crate) fn update(path: &Path, change: SettingsChange) -> Result<Settings, Report> {
    let dir = path
        .parent()
        .ok_or_else(|| report!("{} has no directory", path.display()))?;
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(LOCK))?;
    let _locked = Flock::lock(lock, FlockArg::LockExclusive)
        .map_err(|(_, error)| report!(error).context("could not lock the settings"))?;
    let settings = load(path)?.with(change);
    save(path, &settings)?;
    Ok(settings)
}

/// Writes `settings` to `path`, replacing the file whole: a new file of
/// mode 0600, named for this write alone, renamed over it, in a directory
/// created 0700.
pub(crate) fn save(path: &Path, settings: &Settings) -> Result<(), Report> {
    let dir = path
        .parent()
        .ok_or_else(|| report!("{} has no directory", path.display()))?;
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let temporary = dir.join(format!(".{FILE}.{}.new", Uuid::new_v4()));
    let mut json = serde_json::to_vec_pretty(settings)?;
    json.push(b'\n');
    let written = write_new(&temporary, &json).and_then(|()| {
        Ok(fs::rename(&temporary, path)
            .context_with(|| format!("could not write {}", path.display()))?)
    });
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Creates `path` (it must not exist yet), mode 0600, holding `bytes`.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Report> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context_with(|| format!("could not create {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
        path::Path,
    };

    use std::thread;

    use crate::{
        adapters::settings::{load, save, update},
        core::settings::{Settings, SettingsChange, Switch},
    };

    #[test]
    fn written_0600_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jampot/settings.json");
        assert_eq!(load(&path).unwrap(), Settings::default());

        let change: SettingsChange = serde_json::from_str(
            r#"{"name":"alice","keys":{"commentMode":"k","showCursor":"F2"},"showCursor":"on"}"#,
        )
        .unwrap();
        let settings = Settings::default().with(change);
        save(&path, &settings).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let read = load(&path).unwrap();
        assert_eq!(read, settings);
        assert_eq!(read.show_cursor, Switch::On);

        // Saving again replaces the file, still 0600.
        save(&path, &Settings::default()).unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert_eq!(mode(&path), 0o600);

        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        let error = format!("{:?}", load(&path).unwrap_err());
        assert!(error.contains("chmod 600"), "{error}");
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "nonsense").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn concurrent_updates_keep_every_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jampot/settings.json");
        let changes = [
            r#"{"name":"dora"}"#,
            r#"{"showCursor":"on"}"#,
            r#"{"commentMode":"off"}"#,
            r#"{"keys":{"commentMode":"k","showCursor":"x"}}"#,
        ];
        thread::scope(|scope| {
            for change in changes {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..10 {
                        let change: SettingsChange = serde_json::from_str(change).unwrap();
                        update(path, change).unwrap();
                    }
                });
            }
        });
        let saved = load(&path).unwrap();
        assert_eq!(saved.name.map(String::from).as_deref(), Some("dora"));
        assert_eq!(saved.show_cursor, Switch::On);
        assert_eq!(saved.comment_mode, Switch::Off);
        let keys = serde_json::to_string(&saved.keys).unwrap();
        assert_eq!(keys, r#"{"commentMode":"k","showCursor":"x"}"#);
        // No temporary file is left behind.
        let mut names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["settings.json", "settings.lock"]);
    }
}
