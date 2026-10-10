//! Caches for what the sources read, one file per source in the project's user cache folder.
//!
//! Each file carries a format version and a key (a fingerprint of what it was read from); a
//! file whose version or key differs is ignored, never trusted. Files are written atomically, so
//! a crash leaves the old cache or the new one, and only the user can read them.

use std::path::{Path, PathBuf};

use rkyv::api::high::{HighSerializer, HighValidator};
use rkyv::bytecheck::CheckBytes;
use rkyv::de::Pool;
use rkyv::rancor::{Error, Strategy};
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::{Archive, Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"STUDSRC1";
/// Magic, format version (u32), padding (u32), key (u64).
const HEADER: usize = 24;

/// The cache folder of one project's sources.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store for the project at `root`, next to its graph cache.
    pub fn for_project(root: &Path) -> Self {
        Self::in_dir(studio_parser::cache::user_cache_dir_for_project(root).join("sources"))
    }

    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.rkyv"))
    }

    /// Saves `value` as `name`, stamped with its format `version` and the `key` it was read from.
    pub fn save<T>(&self, name: &str, version: u32, key: u64, value: &T) -> std::io::Result<()>
    where
        T: for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, Error>>,
    {
        let payload = rkyv::to_bytes::<Error>(value).map_err(std::io::Error::other)?;
        let mut bytes = Vec::with_capacity(HEADER + payload.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&payload);
        create_private_dir(&self.dir)?;
        studio_parser::atomic_write(&self.path(name), &bytes)
    }

    /// Loads `name` if it exists with this `version` and `key` and reads back intact.
    pub fn load<T>(&self, name: &str, version: u32, key: u64) -> Option<T>
    where
        T: Archive,
        T::Archived: for<'a> CheckBytes<HighValidator<'a, Error>> + Deserialize<T, Strategy<Pool, Error>>,
    {
        let bytes = std::fs::read(self.path(name)).ok()?;
        let header = bytes.get(..HEADER)?;
        let stored_version = u32::from_le_bytes(header[8..12].try_into().ok()?);
        let stored_key = u64::from_le_bytes(header[16..24].try_into().ok()?);
        if &header[..8] != MAGIC || stored_version != version || stored_key != key {
            return None;
        }
        // rkyv needs the payload aligned; a plain read gives no such promise.
        let mut aligned = AlignedVec::<16>::with_capacity(bytes.len() - HEADER);
        aligned.extend_from_slice(&bytes[HEADER..]);
        rkyv::from_bytes::<T, Error>(&aligned).ok()
    }
}

/// Creates `dir` (and its parents) readable only by the user.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
    struct Branches {
        names: Vec<String>,
        head: Option<u32>,
    }

    #[test]
    fn a_value_comes_back_only_with_the_same_version_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_dir(dir.path().join("sources"));
        let value = Branches { names: vec!["main".into(), "feature".into()], head: Some(1) };
        store.save("git", 3, 42, &value).unwrap();

        assert_eq!(store.load::<Branches>("git", 3, 42), Some(value));
        assert_eq!(store.load::<Branches>("git", 4, 42), None, "another format version");
        assert_eq!(store.load::<Branches>("git", 3, 43), None, "read from something else");
        assert_eq!(store.load::<Branches>("tools", 3, 42), None, "never saved");
    }

    #[test]
    fn a_damaged_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_dir(dir.path());
        store.save("git", 1, 7, &Branches { names: vec!["main".into()], head: None }).unwrap();
        let path = dir.path().join("git.rkyv");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 3);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(store.load::<Branches>("git", 1, 7), None);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_user_can_read_the_cache() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_dir(dir.path().join("sources"));
        store.save("agents", 1, 1, &Branches { names: vec![], head: None }).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join("sources")), 0o700);
        assert_eq!(mode(&dir.path().join("sources/agents.rkyv")), 0o600);
    }
}
