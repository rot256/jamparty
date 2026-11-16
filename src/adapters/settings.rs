//! The joiner's settings file, `~/.config/jamparty/settings.json` (or under
//! `$XDG_CONFIG_HOME`): the defaults each join starts from. jamparty only
//! reads it; the person writes it. Changes made in a page hold for that
//! join only, and the page keeps them for its share (docs/ui.md). A file
//! open to other users is refused.

use std::{
    env, fs,
    io::ErrorKind,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use rootcause::{
    option_ext::OptionExt,
    prelude::{ResultExt, bail, report},
};

use crate::{Report, core::settings::Settings};

const FILE: &str = "settings.json";
/// Settings files are small; a larger one is not ours.
const MAX_BYTES: u64 = 64 * 1024;

/// jamparty's configuration directory (`~/.config/jamparty`, or under
/// `$XDG_CONFIG_HOME`), from the environment.
fn config_dir() -> Result<PathBuf, Report> {
    let config = match env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = env::var_os("HOME").context("HOME is not set")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(config.join("jamparty"))
}

/// Where the settings live, from the environment.
pub(crate) fn default_path() -> Result<PathBuf, Report> {
    Ok(config_dir()?.join(FILE))
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

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use crate::{
        adapters::settings::load,
        core::settings::{Settings, Switch},
    };

    #[test]
    fn read_with_defaults_and_refused_when_open_to_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jamparty/settings.json");
        assert_eq!(load(&path).unwrap(), Settings::default());

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"name":"alice","keys":{"commentMode":"k","showCursor":"F2"},"showCursor":"on","answerTimeout":3600}"#,
        )
        .unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        let read = load(&path).unwrap();
        assert_eq!(read.name.map(String::from).as_deref(), Some("alice"));
        assert_eq!(read.show_cursor, Switch::On);
        assert_eq!(u16::from(read.answer_timeout), 3600);

        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        let error = format!("{:?}", load(&path).unwrap_err());
        assert!(error.contains("chmod 600"), "{error}");
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "nonsense").unwrap();
        assert!(load(&path).is_err());
        for text in [
            r#"{"answerTimeout":3601}"#,
            r#"{"keys":{"commentMode":"c","showCursor":"x","clearAnswers":"X"}}"#,
        ] {
            fs::write(&path, text).unwrap();
            assert!(load(&path).is_err());
        }
        // A key binding no page could choose is refused here too.
        fs::write(
            &path,
            r#"{"keys":{"commentMode":"Enter","showCursor":"x"}}"#,
        )
        .unwrap();
        assert!(load(&path).is_err());
    }
}
