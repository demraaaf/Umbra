//! Albums: named groups of clips, stored in `albums.json` in the settings folder. An album
//! only holds paths; the clip files stay where they are.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// The id of the built-in Favorites album, which always exists and cannot be deleted or
/// renamed.
pub const FAVORITES: &str = "favorites";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    /// Clips in this album are not shown on the Clips screen.
    #[serde(default)]
    pub hide_from_clips: bool,
    #[serde(default)]
    pub clip_paths: Vec<String>,
    pub created_at: String,
    /// True for Favorites. Not stored; set when listing.
    #[serde(default, skip_deserializing)]
    pub builtin: bool,
}

fn favorites() -> Album {
    Album {
        id: FAVORITES.into(),
        name: "Favorites".into(),
        hide_from_clips: false,
        clip_paths: Vec::new(),
        created_at: crate::library::iso8601(SystemTime::now()),
        builtin: true,
    }
}

pub struct AlbumStore {
    path: PathBuf,
    albums: Mutex<Vec<Album>>,
}

/// Windows paths differ only in case when they point at the same file.
fn same_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{:x}{:x}", nanos, COUNTER.fetch_add(1, Ordering::Relaxed))
}

impl AlbumStore {
    /// Reads the albums file. A missing or broken file gives no albums.
    pub fn load(path: PathBuf) -> Self {
        let albums = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| match serde_json::from_str::<Vec<Album>>(&text) {
                Ok(albums) => Some(albums),
                Err(err) => {
                    // Kept aside, as the next change replaces it, so the albums can be put back.
                    let kept = path.with_extension("json.unreadable");
                    let _ = std::fs::copy(&path, &kept);
                    log::warn!(
                        "albums file {} is invalid (kept as {}): {err}",
                        path.display(),
                        kept.display()
                    );
                    None
                }
            })
            .unwrap_or_default();
        let mut albums = albums;
        if !albums.iter().any(|a| a.id == FAVORITES) {
            albums.insert(0, favorites());
        }
        Self {
            path,
            albums: Mutex::new(albums),
        }
    }

    fn save(&self, albums: &[Album]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = self.path.with_extension("json.part");
        std::fs::write(&partial, serde_json::to_string_pretty(albums)?)
            .with_context(|| format!("cannot write {}", partial.display()))?;
        std::fs::rename(&partial, &self.path)
            .with_context(|| format!("cannot replace {}", self.path.display()))?;
        Ok(())
    }

    /// Applies a change and stores the result; nothing is kept if storing fails.
    fn change<T>(&self, apply: impl FnOnce(&mut Vec<Album>) -> Result<T>) -> Result<T> {
        let mut albums = self.albums.lock().unwrap();
        let mut next = albums.clone();
        let result = apply(&mut next)?;
        self.save(&next)?;
        *albums = next;
        Ok(result)
    }

    fn find<'a>(albums: &'a mut [Album], id: &str) -> Result<&'a mut Album> {
        albums
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow!("that album no longer exists"))
    }

    /// All albums, Favorites first and then newest first, with paths of deleted files left out.
    pub fn list(&self) -> Vec<Album> {
        let mut albums = self.albums.lock().unwrap().clone();
        for album in &mut albums {
            album.clip_paths.retain(|p| Path::new(p).exists());
            album.builtin = album.id == FAVORITES;
        }
        albums.sort_by(|a, b| {
            b.builtin
                .cmp(&a.builtin)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        albums
    }

    pub fn create(&self, name: &str) -> Result<Album> {
        let name = clean_name(name)?;
        self.change(|albums| {
            let album = Album {
                id: new_id(),
                name,
                hide_from_clips: false,
                clip_paths: Vec::new(),
                created_at: crate::library::iso8601(SystemTime::now()),
                builtin: false,
            };
            albums.push(album.clone());
            Ok(album)
        })
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Album> {
        if id == FAVORITES {
            bail!("Favorites is built in and keeps its name");
        }
        let name = clean_name(name)?;
        self.change(|albums| {
            let album = Self::find(albums, id)?;
            album.name = name;
            Ok(album.clone())
        })
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if id == FAVORITES {
            bail!("Favorites is built in and cannot be deleted");
        }
        self.change(|albums| {
            let before = albums.len();
            albums.retain(|a| a.id != id);
            if albums.len() == before {
                bail!("that album no longer exists");
            }
            Ok(())
        })
    }

    pub fn set_hidden(&self, id: &str, hide: bool) -> Result<Album> {
        self.change(|albums| {
            let album = Self::find(albums, id)?;
            album.hide_from_clips = hide;
            Ok(album.clone())
        })
    }

    pub fn add(&self, id: &str, paths: &[String]) -> Result<Album> {
        self.change(|albums| {
            let album = Self::find(albums, id)?;
            for path in paths {
                if !album.clip_paths.iter().any(|p| same_path(p, path)) {
                    album.clip_paths.push(path.clone());
                }
            }
            Ok(album.clone())
        })
    }

    pub fn remove(&self, id: &str, paths: &[String]) -> Result<Album> {
        self.change(|albums| {
            let album = Self::find(albums, id)?;
            album
                .clip_paths
                .retain(|p| !paths.iter().any(|gone| same_path(p, gone)));
            Ok(album.clone())
        })
    }

    /// Ids of the albums holding `path`.
    pub fn albums_of(&self, path: &str) -> Vec<String> {
        self.albums
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.clip_paths.iter().any(|p| same_path(p, path)))
            .map(|a| a.id.clone())
            .collect()
    }

    /// Keeps albums pointing at a clip that was renamed. Returns whether anything changed.
    pub fn path_renamed(&self, old: &str, new: &str) -> Result<bool> {
        self.change(|albums| {
            let mut changed = false;
            for album in albums.iter_mut() {
                for path in album.clip_paths.iter_mut() {
                    if same_path(path, old) {
                        *path = new.to_string();
                        changed = true;
                    }
                }
            }
            Ok(changed)
        })
    }

    /// Drops deleted clips from every album. Returns whether anything changed.
    pub fn paths_deleted(&self, gone: &[String]) -> Result<bool> {
        self.change(|albums| {
            let mut changed = false;
            for album in albums.iter_mut() {
                let before = album.clip_paths.len();
                album
                    .clip_paths
                    .retain(|p| !gone.iter().any(|g| same_path(p, g)));
                changed |= album.clip_paths.len() != before;
            }
            Ok(changed)
        })
    }
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("an album needs a name");
    }
    Ok(name.chars().take(80).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (AlbumStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("umbra-albums-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (AlbumStore::load(dir.join("albums.json")), dir)
    }

    #[test]
    fn albums_survive_a_reload() {
        let (albums, dir) = store("reload");
        let a = albums.create("  Best of  ").unwrap();
        assert_eq!(a.name, "Best of");
        albums
            .add(
                &a.id,
                &["C:\\Clips\\one.mp4".into(), "C:\\Clips\\two.mp4".into()],
            )
            .unwrap();
        albums.set_hidden(&a.id, true).unwrap();
        let reloaded = AlbumStore::load(dir.join("albums.json"));
        let raw = reloaded.albums.lock().unwrap().clone();
        assert_eq!(raw.len(), 2, "the album and Favorites");
        let best = raw.iter().find(|x| x.id == a.id).unwrap();
        assert!(best.hide_from_clips);
        assert_eq!(best.clip_paths.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn favorites_always_exists_first_and_cannot_be_deleted_or_renamed() {
        let (albums, dir) = store("favorites");
        albums.create("Later").unwrap();
        let list = albums.list();
        assert_eq!(list[0].id, FAVORITES);
        assert!(list[0].builtin && !list[1].builtin);
        assert!(albums.delete(FAVORITES).is_err());
        assert!(albums.rename(FAVORITES, "Mine").is_err());
        albums.add(FAVORITES, &[r"C:\Clips\a.mp4".into()]).unwrap();
        // Kept across a reload, with its clip.
        let reloaded = AlbumStore::load(dir.join("albums.json"));
        let raw = reloaded.albums.lock().unwrap().clone();
        assert_eq!(raw.iter().filter(|x| x.id == FAVORITES).count(), 1);
        assert_eq!(
            raw.iter()
                .find(|x| x.id == FAVORITES)
                .unwrap()
                .clip_paths
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn adding_twice_keeps_one_and_case_does_not_matter() {
        let (albums, dir) = store("dupes");
        let a = albums.create("x").unwrap();
        albums.add(&a.id, &["C:\\Clips\\One.mp4".into()]).unwrap();
        let a = albums.add(&a.id, &["c:\\clips\\one.mp4".into()]).unwrap();
        assert_eq!(a.clip_paths.len(), 1);
        assert_eq!(albums.albums_of("C:\\CLIPS\\ONE.MP4"), vec![a.id.clone()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn renames_and_deletes_follow_the_files() {
        let (albums, dir) = store("follow");
        let a = albums.create("x").unwrap();
        albums
            .add(&a.id, &["C:\\a.mp4".into(), "C:\\b.mp4".into()])
            .unwrap();
        assert!(albums.path_renamed("C:\\a.mp4", "C:\\renamed.mp4").unwrap());
        assert!(albums.paths_deleted(&["C:\\b.mp4".into()]).unwrap());
        let raw = albums.albums.lock().unwrap().clone();
        let x = raw.iter().find(|x| x.id == a.id).unwrap();
        assert_eq!(x.clip_paths, vec!["C:\\renamed.mp4".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_input_is_refused() {
        let (albums, dir) = store("bad");
        assert!(albums.create("   ").is_err());
        assert!(albums.rename("missing", "x").is_err());
        assert!(albums.delete("missing").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
