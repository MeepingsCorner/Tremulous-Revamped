//! `trap_FS_*` for the game module (trem_sim `world::Fs`, dec026): reads
//! search the user folder (user://, where admin.dat and saved layouts live)
//! first, then the pk3 search path; writes go to the user folder only, like
//! fs_homepath in the original.

use crate::assets::Pk3Set;
use godot::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;

pub struct HostFs {
    user: PathBuf,
    pk3: RefCell<Pk3Set>,
}

impl HostFs {
    pub fn new(pk3_paths: &[String]) -> Self {
        let user = godot::classes::ProjectSettings::singleton().globalize_path("user://").to_string();
        HostFs { user: PathBuf::from(user), pk3: RefCell::new(Pk3Set::open(pk3_paths)) }
    }
    fn user_path(&self, path: &str) -> Option<PathBuf> {
        // game paths only: no absolute paths or parent directories
        if path.starts_with(['/', '\\']) || path.split(['/', '\\']).any(|c| c == "..") || path.contains(':') {
            return None;
        }
        Some(self.user.join(path))
    }
}

impl trem_sim::kernel::world::Fs for HostFs {
    fn read(&self, path: &str) -> Option<String> {
        if let Some(t) = self.user_path(path).and_then(|p| std::fs::read(p).ok()) {
            return Some(String::from_utf8_lossy(&t).into_owned());
        }
        let data = self.pk3.borrow_mut().read(path)?;
        Some(String::from_utf8_lossy(data.as_slice()).into_owned())
    }
    fn write(&mut self, path: &str, text: &str) -> bool {
        let Some(p) = self.user_path(path) else { return false };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(p, text).is_ok()
    }
    /// `FS_APPEND`: games.log grows in the user folder.
    fn append(&mut self, path: &str, text: &str) -> bool {
        use std::io::Write;
        let Some(p) = self.user_path(path) else { return false };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::OpenOptions::new().create(true).append(true).open(p).and_then(|mut f| f.write_all(text.as_bytes())).is_ok()
    }
    fn list(&self, dir: &str, ext: &str) -> Vec<String> {
        let ext_l = ext.to_lowercase();
        let mut out: Vec<String> = Vec::new();
        if let Some(rd) = self.user_path(dir).and_then(|p| std::fs::read_dir(p).ok()) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if e.path().is_file() && n.to_lowercase().ends_with(&ext_l) {
                    out.push(n);
                }
            }
        }
        let pre = format!("{}/", dir.trim_end_matches('/').to_lowercase());
        for f in self.pk3.borrow().files() {
            let l = f.to_lowercase();
            if let Some(rest) = l.strip_prefix(&pre) {
                if !rest.contains('/') && rest.ends_with(&ext_l) {
                    out.push(f[pre.len()..].to_string());
                }
            }
        }
        out.sort_by_key(|s| s.to_lowercase());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out
    }
    fn exists(&self, path: &str) -> bool {
        self.user_path(path).is_some_and(|p| p.is_file()) || self.pk3.borrow().has_file(path)
    }
}
