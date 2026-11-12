//! The joiner's persistent iroh key: `~/.config/jampot/key` (or under
//! `$XDG_CONFIG_HOME`), 64 hex characters, mode 0600, created on first use.

use std::{
    env,
    fs::{self, DirBuilder, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use data_encoding::HEXLOWER;
use iroh::SecretKey;
use rootcause::{
    option_ext::OptionExt,
    prelude::{ResultExt, bail, report},
};
use zeroize::Zeroizing;

use crate::{Report, adapters::endpoint::generate_key, core::rng::SystemRng};

/// Where the joiner key lives, from the environment.
pub(crate) fn default_path() -> Result<PathBuf, Report> {
    let config = match env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = env::var_os("HOME").context("HOME is not set")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(config.join("jampot").join("key"))
}

/// Reads the key at `path`, creating it first if it does not exist. A key
/// others may read or write is refused.
pub(crate) fn load_or_create(path: &Path) -> Result<SecretKey, Report> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let text = Zeroizing::new(text);
            let mode = fs::metadata(path)?.mode();
            if mode & 0o077 != 0 {
                bail!(
                    "{} is open to other users (mode {:o}); chmod 600 it, or delete it for a new key",
                    path.display(),
                    mode & 0o777
                );
            }
            Ok(parse(&text).context(path.display().to_string())?)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => create(path),
        Err(error) => Err(report!(error)
            .context(format!("could not read {}", path.display()))
            .into()),
    }
}

/// The key's bytes and its hex form only ever sit in fixed buffers wiped
/// on drop.
const KEY_BYTES: usize = 32;
const KEY_HEX: usize = 2 * KEY_BYTES;

fn parse(text: &str) -> Result<SecretKey, Report> {
    let hex = text.trim().as_bytes();
    if hex.len() != KEY_HEX {
        bail!("a key file holds {KEY_HEX} lowercase hex characters");
    }
    let mut bytes = Zeroizing::new([0; KEY_BYTES]);
    HEXLOWER
        .decode_mut(hex, bytes.as_mut_slice())
        .map_err(|_| report!("a key file holds {KEY_HEX} lowercase hex characters"))?;
    Ok(SecretKey::from_bytes(&bytes))
}

fn create(path: &Path) -> Result<SecretKey, Report> {
    if let Some(dir) = path.parent() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let key = generate_key(&SystemRng);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context_with(|| format!("could not create {}", path.display()))?;
    let bytes = Zeroizing::new(key.to_bytes());
    let mut hex = Zeroizing::new([0; KEY_HEX]);
    HEXLOWER.encode_mut(bytes.as_slice(), hex.as_mut_slice());
    file.write_all(hex.as_slice())?;
    file.write_all(b"\n")?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use crate::adapters::keyfile::load_or_create;

    #[test]
    fn created_once_with_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jampot/key");
        let first = load_or_create(&path).unwrap();
        let again = load_or_create(&path).unwrap();
        assert_eq!(first.public(), again.public());
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        let error = load_or_create(&path).unwrap_err().to_string();
        assert!(error.contains("chmod 600"), "{error}");
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        assert!(load_or_create(&path).is_ok());
        fs::write(&path, "nonsense").unwrap();
        assert!(load_or_create(&path).is_err());
    }
}
