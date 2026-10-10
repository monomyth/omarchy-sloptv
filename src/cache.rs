//! On-disk cache (~/.cache/slop-tv): clips/, posters/, tiles/, videos.json, videos.etag.
//! Total size of clips+posters+tiles is capped; oldest-modified files are evicted first.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
}

/// Pure policy: which files to delete so the total is <= cap. Oldest first; protected files are kept.
pub fn plan_eviction(files: &[Entry], cap: u64, protect: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut total: u64 = files.iter().map(|f| f.size).sum();
    if total <= cap {
        return vec![];
    }
    let mut order: Vec<&Entry> = files
        .iter()
        .filter(|f| !protect.contains(&f.path))
        .collect();
    order.sort_by_key(|f| f.mtime);
    let mut out = vec![];
    for f in order {
        if total <= cap {
            break;
        }
        total -= f.size;
        out.push(f.path.clone());
    }
    out
}

#[derive(Clone)]
pub struct Cache {
    pub root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Cache {
        for d in ["clips", "posters", "tiles"] {
            let _ = std::fs::create_dir_all(root.join(d));
        }
        Cache { root }
    }
    pub fn clip(&self, id: &str) -> PathBuf {
        self.root.join("clips").join(format!("{}.mp4", safe(id)))
    }
    pub fn poster(&self, id: &str) -> PathBuf {
        self.root.join("posters").join(format!("{}.jpg", safe(id)))
    }
    pub fn tile(&self, id: &str, w: u32, h: u32) -> PathBuf {
        self.root
            .join("tiles")
            .join(format!("{}_{}x{}.bgra", safe(id), w, h))
    }
    pub fn feed(&self) -> PathBuf {
        self.root.join("videos.json")
    }
    pub fn etag(&self) -> PathBuf {
        self.root.join("videos.etag")
    }
    pub fn has_clip(&self, id: &str) -> bool {
        std::fs::metadata(self.clip(id))
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    }
    /// Mark a file as recently used (so LRU eviction keeps it).
    pub fn touch(path: &Path) {
        if let Ok(f) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
    pub fn scan(&self) -> Vec<Entry> {
        let mut v = vec![];
        for d in ["clips", "posters", "tiles"] {
            if let Ok(rd) = std::fs::read_dir(self.root.join(d)) {
                for e in rd.flatten() {
                    if let Ok(m) = e.metadata() {
                        if m.is_file() {
                            v.push(Entry {
                                path: e.path(),
                                size: m.len(),
                                mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            });
                        }
                    }
                }
            }
        }
        v
    }
    pub fn size(&self) -> u64 {
        self.scan().iter().map(|f| f.size).sum()
    }
    /// Enforce the cap now; returns number of files removed.
    pub fn enforce_cap(&self, cap_bytes: u64, protect: &HashSet<PathBuf>) -> usize {
        let victims = plan_eviction(&self.scan(), cap_bytes, protect);
        victims
            .iter()
            .filter(|p| std::fs::remove_file(p).is_ok())
            .count()
    }
}

fn safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn e(name: &str, size: u64, age: u64) -> Entry {
        Entry {
            path: PathBuf::from(name),
            size,
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age),
        }
    }
    #[test]
    fn under_cap_keeps_all() {
        assert!(plan_eviction(&[e("a", 10, 1), e("b", 10, 2)], 20, &HashSet::new()).is_empty());
    }
    #[test]
    fn evicts_oldest_first_until_under_cap() {
        let f = [e("new", 40, 1), e("old", 40, 100), e("mid", 40, 50)];
        let v = plan_eviction(&f, 80, &HashSet::new());
        assert_eq!(v, vec![PathBuf::from("old")]);
        let v = plan_eviction(&f, 40, &HashSet::new());
        assert_eq!(v, vec![PathBuf::from("old"), PathBuf::from("mid")]);
    }
    #[test]
    fn protected_files_survive() {
        let f = [e("new", 40, 1), e("old", 40, 100), e("mid", 40, 50)];
        let p: HashSet<_> = [PathBuf::from("old")].into();
        assert_eq!(plan_eviction(&f, 80, &p), vec![PathBuf::from("mid")]);
        // cap unreachable: everything unprotected goes, protected stays
        assert_eq!(plan_eviction(&f, 0, &p).len(), 2);
    }
    #[test]
    fn enforce_cap_on_disk() {
        let dir = std::env::temp_dir().join(format!("slop-tv-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let c = Cache::new(dir.clone());
        for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
            std::fs::write(c.clip(id), vec![0u8; 1000]).unwrap();
            let f = std::fs::OpenOptions::new()
                .append(true)
                .open(c.clip(id))
                .unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(1000 - i as u64 * 100))
                .unwrap();
        }
        assert_eq!(c.size(), 4000);
        // touching 'a' makes it the newest
        Cache::touch(&c.clip("a"));
        let removed = c.enforce_cap(2500, &HashSet::new());
        assert_eq!(removed, 2);
        assert!(c.size() <= 2500);
        assert!(c.has_clip("a"), "recently used survives");
        assert!(c.has_clip("d"));
        assert!(!c.has_clip("b") && !c.has_clip("c"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn ids_are_sanitised() {
        let c = Cache {
            root: PathBuf::from("/r"),
        };
        assert_eq!(c.clip("../x/1"), PathBuf::from("/r/clips/___x_1.mp4"));
    }
}
