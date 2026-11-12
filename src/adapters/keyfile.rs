//! The joiner's persistent iroh key: `~/.config/jamjam/key` (or under
//! `$XDG_CONFIG_HOME`), 64 hex characters, mode 0600, created on first use.

use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use data_encoding::HEXLOWER;
use iroh::SecretKey;

use crate::{BoxError, adapters::endpoint::generate_key, core::rng::SystemRng};

/// Where the joiner key lives, from the environment.
pub(crate) fn default_path() -> Result<PathBuf, BoxError> {
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(config.join("jamjam").join("key"))
}

/// Reads the key at `path`, creating it first if it does not exist.
pub(crate) fn load_or_create(path: &Path) -> Result<SecretKey, BoxError> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text).map_err(|error| format!("{}: {error}", path.display()).into()),
        Err(error) if error.kind() == ErrorKind::NotFound => create(path),
        Err(error) => Err(format!("could not read {}: {error}", path.display()).into()),
    }
}

fn parse(text: &str) -> Result<SecretKey, BoxError> {
    let bytes = HEXLOWER
        .decode(text.trim().as_bytes())
        .map_err(|_| "a key file holds 64 lowercase hex characters")?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "a key file holds 64 lowercase hex characters")?;
    Ok(SecretKey::from_bytes(&bytes))
}

fn create(path: &Path) -> Result<SecretKey, BoxError> {
    if let Some(dir) = path.parent() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let key = generate_key(&SystemRng);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    writeln!(file, "{}", HEXLOWER.encode(&key.to_bytes()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use crate::adapters::keyfile::load_or_create;

    #[test]
    fn created_once_with_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jamjam/key");
        let first = load_or_create(&path).unwrap();
        let again = load_or_create(&path).unwrap();
        assert_eq!(first.public(), again.public());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::write(&path, "nonsense").unwrap();
        assert!(load_or_create(&path).is_err());
    }
}
