//! The on-disk cache: the [`Store`] as JSON, so the next start shows everything immediately and only
//! refetches what has gone stale. Best effort throughout: a missing, corrupt or foreign cache is ignored.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

use crate::data::Store;

/// `~/.cache/blackbaud-tui/cache.json` (or the platform's equivalent). It holds school data, so it is
/// written 0600 in a 0700 directory.
pub fn default_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("blackbaud-tui").join("cache.json"))
}

/// The stored cache, if there is one that belongs to `user` and this version of the program.
pub fn load(path: &Path, user: Option<&str>) -> Option<Store> {
    let store: Store = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    store.usable_for(user).then_some(store)
}

pub fn clear(path: &Path) {
    let _ = fs::remove_file(path);
}

fn write(path: &Path, json: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let tmp = path.with_extension("tmp");
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(json)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// Writes snapshots on its own thread so a slow disk never stalls the screen. Snapshots that pile up
/// while a write is running are collapsed to the newest.
pub struct Writer {
    tx: Sender<Vec<u8>>,
    handle: JoinHandle<()>,
}

impl Writer {
    pub fn spawn(path: PathBuf) -> Writer {
        let (tx, rx) = channel::<Vec<u8>>();
        let handle = std::thread::Builder::new()
            .name("cache-writer".into())
            .spawn(move || {
                while let Ok(mut json) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        json = newer;
                    }
                    let _ = write(&path, &json);
                }
            })
            .expect("spawn cache thread");
        Writer { tx, handle }
    }

    /// Wait for the queued snapshot to reach the disk.
    pub fn finish(self) {
        drop(self.tx);
        let _ = self.handle.join();
    }

    pub fn save(&self, store: &mut Store) {
        store.prune();
        if let Ok(json) = serde_json::to_vec(store) {
            let _ = self.tx.send(json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Key, Payload};

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bbtui-cache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d.join("cache.json")
    }

    fn now() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap().and_hms_opt(9, 0, 0).unwrap()
    }

    #[test]
    fn a_saved_cache_comes_back_private_and_only_for_its_owner() {
        let path = scratch("roundtrip");
        let mut s = Store::new(Some("student".into()));
        s.apply(&Key::Center, Payload::Center(crate::demo::items(now())), now());
        let w = Writer::spawn(path.clone());
        w.save(&mut s);
        w.finish();
        let back = load(&path, Some("student")).expect("cache written");
        assert_eq!(back.items(), s.items());
        assert!(load(&path, Some("someone else")).is_none());
        assert!(load(&path, None).is_some(), "an unknown user takes the cache; the sign-in will tell");
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&path), mode(path.parent().unwrap())), (0o600, 0o700));
        clear(&path);
        assert!(load(&path, None).is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn junk_is_ignored_not_fatal() {
        let path = scratch("junk");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ not json").unwrap();
        assert!(load(&path, None).is_none());
        fs::write(&path, b"{\"version\": 999}").unwrap();
        assert!(load(&path, None).is_none(), "another version's cache is not trusted");
        assert!(load(Path::new("/nonexistent/x.json"), None).is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
