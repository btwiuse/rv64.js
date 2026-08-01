//! Filesystem backends for the 9P server ([`crate::p9::FsBackend`]).
//!
//! [`MemFs`] is an in-memory tree: portable (it is the only backend that
//! compiles for wasm, where there is no host filesystem), deterministic, and
//! the vehicle the protocol tests drive. It can be populated programmatically
//! or from a tar archive, which is how a browser guest gets a root filesystem.
//!
//! [`HostFs`] exports a real host directory, mirroring TinyEMU's `fs_disk.c`.
//! This is the native "mount ~/src in the guest" case.

use crate::p9::*;
use std::collections::BTreeMap;

// ---- in-memory ------------------------------------------------------------

enum Body {
    /// name -> node index
    Dir(BTreeMap<String, usize>),
    File(Vec<u8>),
    Link(String),
    /// Device or fifo: metadata only.
    Special {
        rdev: u64,
    },
    /// Removed. Nodes are tombstoned rather than compacted so that inode
    /// numbers (which the client caches in qids) are never reused.
    Freed,
}

struct Node {
    body: Body,
    mode: u32,
    uid: u32,
    gid: u32,
    atime: (u64, u64),
    mtime: (u64, u64),
    ctime: (u64, u64),
}

/// An in-memory filesystem tree.
pub struct MemFs {
    /// Node 0 is the root; a node's inode number is its index + 1.
    nodes: Vec<Node>,
    /// Reported by statfs as the total size, so a guest sees a finite volume.
    capacity: u64,
}

impl Default for MemFs {
    fn default() -> MemFs {
        MemFs::new()
    }
}

impl MemFs {
    pub fn new() -> MemFs {
        MemFs {
            nodes: vec![Node {
                body: Body::Dir(BTreeMap::new()),
                mode: S_IFDIR | 0o755,
                uid: 0,
                gid: 0,
                atime: (0, 0),
                mtime: (0, 0),
                ctime: (0, 0),
            }],
            capacity: 64 << 20,
        }
    }

    /// Total bytes reported to the guest as the volume size.
    pub fn set_capacity(&mut self, bytes: u64) {
        self.capacity = bytes;
    }

    /// Create `path` and any missing parents, returning the directory's node.
    pub fn add_dir(&mut self, path: &str) -> usize {
        let mut idx = 0;
        for comp in components(path) {
            idx = match self.child_idx(idx, comp) {
                Some(c) => c,
                None => {
                    let new = self.alloc(Body::Dir(BTreeMap::new()), S_IFDIR | 0o755);
                    self.link_child(idx, comp, new);
                    new
                }
            };
        }
        idx
    }

    /// Create or replace a regular file, adding parent directories as needed.
    pub fn add_file(&mut self, path: &str, data: &[u8], mode: u32) {
        let (dir, name) = self.ensure_parent(path);
        let node = self.alloc(Body::File(data.to_vec()), S_IFREG | (mode & 0o7777));
        self.link_child(dir, &name, node);
    }

    pub fn add_symlink(&mut self, path: &str, target: &str) {
        let (dir, name) = self.ensure_parent(path);
        let node = self.alloc(Body::Link(target.to_string()), S_IFLNK | 0o777);
        self.link_child(dir, &name, node);
    }

    /// Load a (ustar or GNU) tar archive into the tree. This is the practical
    /// way to hand a browser guest a root filesystem: fetch a tarball, mount
    /// it over 9p. Entries other than files, directories and symlinks are
    /// skipped. Returns the number of entries loaded.
    pub fn load_tar(&mut self, tar: &[u8]) -> usize {
        let mut pos = 0usize;
        let mut loaded = 0usize;
        // A GNU 'L' record carries the next entry's (over-long) name.
        let mut long_name: Option<String> = None;
        while pos + 512 <= tar.len() {
            let hdr = &tar[pos..pos + 512];
            if hdr.iter().all(|&b| b == 0) {
                break; // end-of-archive marker
            }
            let size = octal(&hdr[124..136]) as usize;
            let mode = octal(&hdr[100..108]) as u32;
            let kind = hdr[156];
            let data_start = pos + 512;
            let data_end = (data_start + size).min(tar.len());
            pos = data_start + size.div_ceil(512) * 512;

            let name = match long_name.take() {
                Some(n) => n,
                None => {
                    let base = cstr(&hdr[0..100]);
                    let prefix = if &hdr[257..262] == b"ustar" {
                        cstr(&hdr[345..500])
                    } else {
                        String::new()
                    };
                    if prefix.is_empty() {
                        base
                    } else {
                        format!("{prefix}/{base}")
                    }
                }
            };
            match kind {
                b'L' => long_name = Some(cstr(&tar[data_start..data_end])),
                b'5' => {
                    self.add_dir(&name);
                    loaded += 1;
                }
                b'0' | 0 => {
                    self.add_file(&name, &tar[data_start..data_end], mode);
                    loaded += 1;
                }
                b'2' => {
                    self.add_symlink(&name, &cstr(&hdr[157..257]));
                    loaded += 1;
                }
                _ => {}
            }
        }
        loaded
    }

    // -- internals --

    fn alloc(&mut self, body: Body, mode: u32) -> usize {
        self.nodes.push(Node {
            body,
            mode,
            uid: 0,
            gid: 0,
            atime: (0, 0),
            mtime: (0, 0),
            ctime: (0, 0),
        });
        self.nodes.len() - 1
    }

    fn child_idx(&self, dir: usize, name: &str) -> Option<usize> {
        match &self.nodes[dir].body {
            Body::Dir(m) => m.get(name).copied(),
            _ => None,
        }
    }

    fn link_child(&mut self, dir: usize, name: &str, node: usize) {
        if let Body::Dir(m) = &mut self.nodes[dir].body {
            m.insert(name.to_string(), node);
        }
    }

    /// Directory node + final component for `path`, creating parents.
    fn ensure_parent(&mut self, path: &str) -> (usize, String) {
        let comps: Vec<&str> = components(path).collect();
        let (name, dirs) = comps.split_last().expect("path has a final component");
        let mut idx = 0;
        for c in dirs {
            idx = match self.child_idx(idx, c) {
                Some(i) => i,
                None => {
                    let new = self.alloc(Body::Dir(BTreeMap::new()), S_IFDIR | 0o755);
                    self.link_child(idx, c, new);
                    new
                }
            };
        }
        (idx, name.to_string())
    }

    fn lookup(&self, path: &str) -> Result<usize, i32> {
        let mut idx = 0usize;
        for comp in components(path) {
            match &self.nodes[idx].body {
                Body::Dir(m) => idx = *m.get(comp).ok_or(ENOENT)?,
                _ => return Err(ENOTDIR),
            }
        }
        Ok(idx)
    }

    /// (parent node, final component) for an existing-or-to-be-created path.
    fn split(&self, path: &str) -> Result<(usize, String), i32> {
        let comps: Vec<&str> = components(path).collect();
        let (name, dirs) = comps.split_last().ok_or(EINVAL)?;
        let mut idx = 0usize;
        for c in dirs {
            match &self.nodes[idx].body {
                Body::Dir(m) => idx = *m.get(*c).ok_or(ENOENT)?,
                _ => return Err(ENOTDIR),
            }
        }
        Ok((idx, name.to_string()))
    }

    fn attr_of(&self, idx: usize) -> Attr {
        let n = &self.nodes[idx];
        let size = match &n.body {
            Body::File(d) => d.len() as u64,
            Body::Link(t) => t.len() as u64,
            Body::Dir(m) => m.len() as u64 * 32,
            _ => 0,
        };
        let rdev = match &n.body {
            Body::Special { rdev } => *rdev,
            _ => 0,
        };
        Attr {
            qid: Qid::from_mode(n.mode, idx as u64 + 1),
            mode: n.mode,
            uid: n.uid,
            gid: n.gid,
            nlink: if n.mode & S_IFMT == S_IFDIR { 2 } else { 1 },
            rdev,
            size,
            blksize: 4096,
            blocks: size.div_ceil(512),
            atime: n.atime,
            mtime: n.mtime,
            ctime: n.ctime,
        }
    }

    /// Insert a fresh node under `path`, failing if the name is taken.
    fn create_at(&mut self, path: &str, body: Body, mode: u32) -> Result<Attr, i32> {
        let (dir, name) = self.split(path)?;
        if self.child_idx(dir, &name).is_some() {
            return Err(EEXIST);
        }
        if !matches!(self.nodes[dir].body, Body::Dir(_)) {
            return Err(ENOTDIR);
        }
        let node = self.alloc(body, mode);
        self.link_child(dir, &name, node);
        Ok(self.attr_of(node))
    }

    fn bytes_used(&self) -> u64 {
        self.nodes
            .iter()
            .map(|n| match &n.body {
                Body::File(d) => d.len() as u64,
                _ => 0,
            })
            .sum()
    }
}

impl FsBackend for MemFs {
    fn statfs(&mut self) -> StatFs {
        let used = self.bytes_used();
        let blocks = self.capacity / 4096;
        let free = self.capacity.saturating_sub(used) / 4096;
        StatFs {
            bsize: 4096,
            blocks,
            bfree: free,
            bavail: free,
            files: self.nodes.len() as u64,
            ffree: 1 << 20,
        }
    }

    fn lstat(&mut self, path: &str) -> Result<Attr, i32> {
        let idx = self.lookup(path)?;
        Ok(self.attr_of(idx))
    }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, i32> {
        let idx = self.lookup(path)?;
        match &self.nodes[idx].body {
            Body::Dir(m) => Ok(m
                .iter()
                .map(|(name, &i)| DirEntry {
                    name: name.clone(),
                    ino: i as u64 + 1,
                    mode: self.nodes[i].mode,
                })
                .collect()),
            _ => Err(ENOTDIR),
        }
    }

    fn open(&mut self, path: &str, flags: u32) -> Result<Attr, i32> {
        let idx = self.lookup(path)?;
        if flags & O_TRUNC != 0 {
            if let Body::File(d) = &mut self.nodes[idx].body {
                d.clear();
            }
        }
        Ok(self.attr_of(idx))
    }

    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i32> {
        let idx = self.lookup(path)?;
        match &self.nodes[idx].body {
            Body::File(d) => {
                let start = (offset as usize).min(d.len());
                let n = buf.len().min(d.len() - start);
                buf[..n].copy_from_slice(&d[start..start + n]);
                Ok(n)
            }
            Body::Dir(_) => Err(EISDIR),
            _ => Err(EINVAL),
        }
    }

    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<usize, i32> {
        let idx = self.lookup(path)?;
        match &mut self.nodes[idx].body {
            Body::File(d) => {
                let start = offset as usize;
                if start + data.len() > d.len() {
                    d.resize(start + data.len(), 0);
                }
                d[start..start + data.len()].copy_from_slice(data);
                Ok(data.len())
            }
            Body::Dir(_) => Err(EISDIR),
            _ => Err(EINVAL),
        }
    }

    fn create(&mut self, path: &str, _flags: u32, mode: u32) -> Result<Attr, i32> {
        self.create_at(path, Body::File(Vec::new()), S_IFREG | (mode & 0o7777))
    }

    fn mkdir(&mut self, path: &str, mode: u32) -> Result<Attr, i32> {
        self.create_at(path, Body::Dir(BTreeMap::new()), S_IFDIR | (mode & 0o7777))
    }

    fn symlink(&mut self, path: &str, target: &str) -> Result<Attr, i32> {
        self.create_at(path, Body::Link(target.to_string()), S_IFLNK | 0o777)
    }

    fn mknod(&mut self, path: &str, mode: u32, major: u32, minor: u32) -> Result<Attr, i32> {
        let rdev = makedev(major, minor);
        self.create_at(path, Body::Special { rdev }, mode)
    }

    fn readlink(&mut self, path: &str) -> Result<String, i32> {
        let idx = self.lookup(path)?;
        match &self.nodes[idx].body {
            Body::Link(t) => Ok(t.clone()),
            _ => Err(EINVAL),
        }
    }

    fn hardlink(&mut self, existing: &str, new: &str) -> Result<(), i32> {
        let src = self.lookup(existing)?;
        let (dir, name) = self.split(new)?;
        if self.child_idx(dir, &name).is_some() {
            return Err(EEXIST);
        }
        self.link_child(dir, &name, src);
        Ok(())
    }

    fn remove(&mut self, path: &str, _is_dir: bool) -> Result<(), i32> {
        let (dir, name) = self.split(path)?;
        let idx = self.child_idx(dir, &name).ok_or(ENOENT)?;
        if let Body::Dir(m) = &self.nodes[idx].body {
            if !m.is_empty() {
                return Err(ENOTEMPTY);
            }
        }
        if let Body::Dir(m) = &mut self.nodes[dir].body {
            m.remove(&name);
        }
        // Tombstone rather than reuse: see Body::Freed.
        self.nodes[idx].body = Body::Freed;
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), i32> {
        let (from_dir, from_name) = self.split(from)?;
        let idx = self.child_idx(from_dir, &from_name).ok_or(ENOENT)?;
        let (to_dir, to_name) = self.split(to)?;
        if let Body::Dir(m) = &mut self.nodes[from_dir].body {
            m.remove(&from_name);
        }
        self.link_child(to_dir, &to_name, idx);
        Ok(())
    }

    fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), i32> {
        let idx = self.lookup(path)?;
        let kind = self.nodes[idx].mode & S_IFMT;
        self.nodes[idx].mode = kind | (mode & 0o7777);
        Ok(())
    }

    fn set_owner(&mut self, path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), i32> {
        let idx = self.lookup(path)?;
        if let Some(u) = uid {
            self.nodes[idx].uid = u;
        }
        if let Some(g) = gid {
            self.nodes[idx].gid = g;
        }
        Ok(())
    }

    fn truncate(&mut self, path: &str, size: u64) -> Result<(), i32> {
        let idx = self.lookup(path)?;
        match &mut self.nodes[idx].body {
            Body::File(d) => {
                d.resize(size as usize, 0);
                Ok(())
            }
            _ => Err(EINVAL),
        }
    }

    fn set_times(
        &mut self,
        path: &str,
        atime: Option<(u64, u64)>,
        mtime: Option<(u64, u64)>,
    ) -> Result<(), i32> {
        let idx = self.lookup(path)?;
        if let Some(a) = atime {
            self.nodes[idx].atime = a;
        }
        if let Some(m) = mtime {
            self.nodes[idx].mtime = m;
        }
        Ok(())
    }
}

/// Non-empty path components, accepting both `/a/b` and `a/b`.
fn components(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|c| !c.is_empty() && *c != ".")
}

fn octal(field: &[u8]) -> u64 {
    field
        .iter()
        .take_while(|&&b| b.is_ascii_digit())
        .fold(0u64, |acc, &b| acc * 8 + (b - b'0') as u64)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Linux `makedev` encoding for `st_rdev`.
fn makedev(major: u32, minor: u32) -> u64 {
    let (ma, mi) = (major as u64, minor as u64);
    ((ma & 0xffff_f000) << 32) | ((ma & 0xfff) << 8) | ((mi & 0xffff_ff00) << 12) | (mi & 0xff)
}

// ---- host directory -------------------------------------------------------

#[cfg(unix)]
pub use host::HostFs;

#[cfg(unix)]
mod host {
    use super::*;
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    /// Exports a host directory over 9P.
    pub struct HostFs {
        root: PathBuf,
        /// Handles established by `open`/`create`, keyed by export-relative
        /// path. 9P read/write carry explicit offsets so no seek state is
        /// needed — but keeping the descriptor is what stops every 4 KiB
        /// request from reopening the file, which is the difference between a
        /// usable mount and a crawl. `bool` records whether it is writable.
        open: HashMap<String, (File, bool)>,
    }

    impl HostFs {
        pub fn new(root: impl Into<PathBuf>) -> HostFs {
            HostFs {
                root: root.into(),
                open: HashMap::new(),
            }
        }

        /// Host path for an export-relative path. The server has already
        /// stripped `.`/`..`, so this cannot escape `root`. Symlinks *inside*
        /// the export are resolved by the guest, one component at a time
        /// through us, so they cannot escape either — except via a direct
        /// `open` of a symlink fid, which `open` below refuses.
        fn real(&self, path: &str) -> PathBuf {
            if path.is_empty() {
                self.root.clone()
            } else {
                self.root.join(&path[1..])
            }
        }

        /// Cached handle for `path`, opening one if needed. Upgrades a
        /// read-only handle when a write arrives.
        fn handle(&mut self, path: &str, write: bool) -> Result<&File, i32> {
            let need_open = match self.open.get(path) {
                Some((_, w)) => write && !w,
                None => true,
            };
            if need_open {
                let f = OpenOptions::new()
                    .read(true)
                    .write(write)
                    .open(self.real(path))
                    .map_err(errno)?;
                self.open.insert(path.to_string(), (f, write));
            }
            Ok(&self.open.get(path).expect("just inserted").0)
        }
    }

    impl FsBackend for HostFs {
        fn statfs(&mut self) -> StatFs {
            // std exposes no statfs, and hardcoding the kernel struct layout to
            // get one would be a poor trade: nothing depends on exact free
            // space, and over-reporting is the safe direction for guests that
            // check before writing.
            StatFs {
                bsize: 4096,
                blocks: 1 << 28,
                bfree: 1 << 27,
                bavail: 1 << 27,
                files: 1 << 20,
                ffree: 1 << 19,
            }
        }

        fn lstat(&mut self, path: &str) -> Result<Attr, i32> {
            let md = std::fs::symlink_metadata(self.real(path)).map_err(errno)?;
            Ok(attr_from(&md))
        }

        fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, i32> {
            let mut out = Vec::new();
            for e in std::fs::read_dir(self.real(path)).map_err(errno)? {
                let e = e.map_err(errno)?;
                // A dangling symlink still belongs in the listing, so fall back
                // to "regular file" rather than dropping the entry.
                let (ino, mode) = match e.metadata() {
                    Ok(md) => (md.ino(), md.mode()),
                    Err(_) => (0, S_IFREG | 0o644),
                };
                out.push(DirEntry {
                    name: e.file_name().to_string_lossy().into_owned(),
                    ino,
                    mode,
                });
            }
            Ok(out)
        }

        fn open(&mut self, path: &str, flags: u32) -> Result<Attr, i32> {
            let real = self.real(path);
            let md = std::fs::symlink_metadata(&real).map_err(errno)?;
            if md.file_type().is_symlink() {
                // The client walks symlinks itself, so it has no business
                // opening one; refusing keeps a host-absolute target from
                // being followed out of the export. ELOOP is what O_NOFOLLOW
                // would report.
                return Err(40);
            }
            if md.is_dir() {
                return Ok(attr_from(&md)); // directories need no handle
            }
            let writable = flags & (O_WRONLY | O_RDWR) != 0;
            let f = OpenOptions::new()
                .read(true)
                .write(writable)
                .truncate(writable && flags & O_TRUNC != 0)
                .open(&real)
                .map_err(errno)?;
            self.open.insert(path.to_string(), (f, writable));
            let md = std::fs::symlink_metadata(&real).map_err(errno)?;
            Ok(attr_from(&md))
        }

        fn close(&mut self, path: &str) {
            self.open.remove(path);
        }

        fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i32> {
            self.handle(path, false)?
                .read_at(buf, offset)
                .map_err(errno)
        }

        fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<usize, i32> {
            self.handle(path, true)?
                .write_at(data, offset)
                .map_err(errno)
        }

        fn create(&mut self, path: &str, flags: u32, mode: u32) -> Result<Attr, i32> {
            let real = self.real(path);
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(flags & O_TRUNC != 0)
                .mode(mode & 0o7777)
                .open(&real)
                .map_err(errno)?;
            self.open.insert(path.to_string(), (f, true));
            let md = std::fs::symlink_metadata(&real).map_err(errno)?;
            Ok(attr_from(&md))
        }

        fn mkdir(&mut self, path: &str, mode: u32) -> Result<Attr, i32> {
            let real = self.real(path);
            std::fs::create_dir(&real).map_err(errno)?;
            let _ = std::fs::set_permissions(&real, PermissionsExt::from_mode(mode & 0o7777));
            self.lstat(path)
        }

        fn symlink(&mut self, path: &str, target: &str) -> Result<Attr, i32> {
            std::os::unix::fs::symlink(target, self.real(path)).map_err(errno)?;
            self.lstat(path)
        }

        fn mknod(&mut self, path: &str, mode: u32, major: u32, minor: u32) -> Result<Attr, i32> {
            let c = cpath(&self.real(path))?;
            // Only fifos and sockets succeed unprivileged; device nodes need
            // CAP_MKNOD, and the host's EPERM is the honest answer there.
            let rc = unsafe { sys::mknod(c.as_ptr(), mode, makedev(major, minor)) };
            if rc != 0 {
                return Err(last_errno());
            }
            self.lstat(path)
        }

        fn readlink(&mut self, path: &str) -> Result<String, i32> {
            let t = std::fs::read_link(self.real(path)).map_err(errno)?;
            Ok(t.to_string_lossy().into_owned())
        }

        fn hardlink(&mut self, existing: &str, new: &str) -> Result<(), i32> {
            std::fs::hard_link(self.real(existing), self.real(new)).map_err(errno)
        }

        fn remove(&mut self, path: &str, is_dir: bool) -> Result<(), i32> {
            self.open.remove(path);
            let real = self.real(path);
            if is_dir {
                std::fs::remove_dir(real).map_err(errno)
            } else {
                std::fs::remove_file(real).map_err(errno)
            }
        }

        fn rename(&mut self, from: &str, to: &str) -> Result<(), i32> {
            self.open.remove(from);
            self.open.remove(to);
            std::fs::rename(self.real(from), self.real(to)).map_err(errno)
        }

        fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), i32> {
            std::fs::set_permissions(self.real(path), PermissionsExt::from_mode(mode & 0o7777))
                .map_err(errno)
        }

        fn set_owner(&mut self, path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), i32> {
            let md = std::fs::symlink_metadata(self.real(path)).map_err(errno)?;
            // A no-op chown must succeed even unprivileged: the guest issues
            // one on some file creations, and failing it would fail the create.
            if uid.is_none_or(|u| u == md.uid()) && gid.is_none_or(|g| g == md.gid()) {
                return Ok(());
            }
            let c = cpath(&self.real(path))?;
            let rc = unsafe {
                sys::lchown(
                    c.as_ptr(),
                    uid.unwrap_or(u32::MAX), // -1 = leave unchanged
                    gid.unwrap_or(u32::MAX),
                )
            };
            if rc != 0 {
                return Err(last_errno());
            }
            Ok(())
        }

        fn truncate(&mut self, path: &str, size: u64) -> Result<(), i32> {
            self.handle(path, true)?.set_len(size).map_err(errno)
        }

        fn set_times(
            &mut self,
            path: &str,
            atime: Option<(u64, u64)>,
            mtime: Option<(u64, u64)>,
        ) -> Result<(), i32> {
            use std::time::{Duration, SystemTime};
            let f = OpenOptions::new()
                .write(true)
                .open(self.real(path))
                .or_else(|_| File::open(self.real(path)))
                .map_err(errno)?;
            let mut times = std::fs::FileTimes::new();
            // `None` means the client asked for "now" (no *_SET bit); a field
            // left unset on FileTimes is left unchanged on disk.
            let to_sys = |(s, ns): (u64, u64)| {
                SystemTime::UNIX_EPOCH + Duration::new(s, ns.min(999_999_999) as u32)
            };
            times = times.set_accessed(atime.map_or_else(SystemTime::now, to_sys));
            times = times.set_modified(mtime.map_or_else(SystemTime::now, to_sys));
            f.set_times(times).map_err(errno)
        }
    }

    fn attr_from(md: &std::fs::Metadata) -> Attr {
        Attr {
            qid: Qid::from_mode(md.mode(), md.ino()),
            mode: md.mode(),
            uid: md.uid(),
            gid: md.gid(),
            nlink: md.nlink(),
            rdev: md.rdev(),
            size: md.size(),
            blksize: md.blksize(),
            blocks: md.blocks(),
            atime: (md.atime() as u64, md.atime_nsec() as u64),
            mtime: (md.mtime() as u64, md.mtime_nsec() as u64),
            ctime: (md.ctime() as u64, md.ctime_nsec() as u64),
        }
    }

    /// 9P2000.L carries Linux errnos and the host *is* Linux, so the host
    /// errno passes straight through.
    fn errno(e: std::io::Error) -> i32 {
        e.raw_os_error().unwrap_or(EIO)
    }

    fn last_errno() -> i32 {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(EIO)
    }

    fn cpath(p: &Path) -> Result<std::ffi::CString, i32> {
        std::ffi::CString::new(p.to_string_lossy().as_bytes()).map_err(|_| EINVAL)
    }

    mod sys {
        extern "C" {
            pub fn mknod(path: *const std::ffi::c_char, mode: u32, dev: u64) -> i32;
            pub fn lchown(path: *const std::ffi::c_char, uid: u32, gid: u32) -> i32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- MemFs / tar ----

    /// One ustar record (header + padded data).
    fn tar_entry(name: &str, kind: u8, data: &[u8], link: &str) -> Vec<u8> {
        fn oct(h: &mut [u8], off: usize, width: usize, v: u64) {
            let s = format!("{:0w$o}", v, w = width - 1);
            h[off..off + s.len()].copy_from_slice(s.as_bytes());
        }
        let mut out = vec![0u8; 512];
        out[..name.len()].copy_from_slice(name.as_bytes());
        oct(&mut out, 100, 8, 0o644); // mode
        oct(&mut out, 124, 12, data.len() as u64); // size
        out[156] = kind;
        out[157..157 + link.len()].copy_from_slice(link.as_bytes());
        out[257..262].copy_from_slice(b"ustar");
        out.extend_from_slice(data);
        out.resize(512 + data.len().div_ceil(512) * 512, 0);
        out
    }

    #[test]
    fn memfs_loads_a_tar_archive() {
        let mut tar = Vec::new();
        tar.extend(tar_entry("bin/", b'5', b"", ""));
        tar.extend(tar_entry("bin/hello", b'0', b"#!/bin/sh\necho hi\n", ""));
        tar.extend(tar_entry("bin/sh", b'2', b"", "busybox"));
        tar.extend(tar_entry("empty", b'0', b"", ""));
        tar.extend(vec![0u8; 1024]); // end-of-archive

        let mut fs = MemFs::new();
        assert_eq!(fs.load_tar(&tar), 4);
        assert_eq!(fs.lstat("/bin").unwrap().mode & S_IFMT, S_IFDIR);
        let hello = fs.lstat("/bin/hello").unwrap();
        assert_eq!(hello.size, 18);
        assert_eq!(hello.mode, S_IFREG | 0o644);
        let mut buf = [0u8; 32];
        let n = fs.read("/bin/hello", 0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"#!/bin/sh\necho hi\n");
        assert_eq!(fs.readlink("/bin/sh").unwrap(), "busybox");
        assert_eq!(fs.lstat("/empty").unwrap().size, 0);
        // Every node gets a distinct inode, since clients cache qids.
        assert_ne!(fs.lstat("/bin").unwrap().qid.path, hello.qid.path);
    }

    #[test]
    fn memfs_inodes_are_never_reused() {
        let mut fs = MemFs::new();
        fs.add_file("/a", b"x", 0o644);
        let first = fs.lstat("/a").unwrap().qid.path;
        fs.remove("/a", false).unwrap();
        fs.create("/b", 0, 0o644).unwrap();
        // A client that cached the old qid must not see it point at new data.
        assert_ne!(fs.lstat("/b").unwrap().qid.path, first);
    }

    #[test]
    fn memfs_rejects_removing_a_populated_directory() {
        let mut fs = MemFs::new();
        fs.add_file("/d/inner", b"x", 0o644);
        assert_eq!(fs.remove("/d", true), Err(ENOTEMPTY));
        fs.remove("/d/inner", false).unwrap();
        assert!(fs.remove("/d", true).is_ok());
    }

    // ---- HostFs ----

    #[cfg(unix)]
    mod host_backend {
        use super::*;

        /// A temp directory that cleans itself up.
        struct Tmp(std::path::PathBuf);
        impl Tmp {
            fn new(name: &str) -> Tmp {
                let p =
                    std::env::temp_dir().join(format!("rv64-p9fs-{}-{name}", std::process::id()));
                let _ = std::fs::remove_dir_all(&p);
                std::fs::create_dir_all(&p).expect("create temp dir");
                Tmp(p)
            }
        }
        impl Drop for Tmp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn creates_reads_and_renames_on_the_host() {
            let tmp = Tmp::new("rename");
            let mut fs = HostFs::new(&tmp.0);

            fs.create("/note.txt", O_RDWR, 0o644).unwrap();
            assert_eq!(fs.write("/note.txt", 0, b"hello host").unwrap(), 10);
            let mut buf = [0u8; 32];
            let n = fs.read("/note.txt", 0, &mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello host");
            assert_eq!(fs.lstat("/note.txt").unwrap().size, 10);
            // Offsets address the file directly — no seek state per handle.
            assert_eq!(fs.write("/note.txt", 6, b"world").unwrap(), 5);
            assert_eq!(
                std::fs::read(tmp.0.join("note.txt")).unwrap(),
                b"hello world"
            );

            // Trenameat, which the ancient TinyEMU guest userland cannot issue.
            fs.mkdir("/sub", 0o755).unwrap();
            fs.rename("/note.txt", "/sub/moved.txt").unwrap();
            assert!(!tmp.0.join("note.txt").exists());
            assert_eq!(
                std::fs::read_to_string(tmp.0.join("sub/moved.txt")).unwrap(),
                "hello world"
            );
            assert_eq!(fs.lstat("/note.txt"), Err(ENOENT));

            // Renaming over an existing file replaces it.
            fs.create("/other.txt", O_RDWR, 0o644).unwrap();
            fs.write("/other.txt", 0, b"second").unwrap();
            fs.rename("/other.txt", "/sub/moved.txt").unwrap();
            assert_eq!(
                std::fs::read_to_string(tmp.0.join("sub/moved.txt")).unwrap(),
                "second"
            );
        }

        #[test]
        fn truncate_mode_and_readdir() {
            let tmp = Tmp::new("attrs");
            let mut fs = HostFs::new(&tmp.0);
            fs.create("/f", O_RDWR, 0o600).unwrap();
            fs.write("/f", 0, b"0123456789").unwrap();
            fs.truncate("/f", 4).unwrap();
            assert_eq!(fs.lstat("/f").unwrap().size, 4);
            assert_eq!(std::fs::read(tmp.0.join("f")).unwrap(), b"0123");

            fs.set_mode("/f", 0o640).unwrap();
            assert_eq!(fs.lstat("/f").unwrap().mode & 0o777, 0o640);

            fs.mkdir("/d", 0o755).unwrap();
            let names: Vec<String> = fs
                .readdir("")
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert!(names.contains(&"f".to_string()));
            assert!(names.contains(&"d".to_string()));
            // The backend does NOT emit "." or ".."; the server synthesises them.
            assert!(!names.contains(&".".to_string()));

            // O_TRUNC on open must empty the file.
            fs.write("/f", 0, b"refilled").unwrap();
            fs.open("/f", O_RDWR | O_TRUNC).unwrap();
            assert_eq!(fs.lstat("/f").unwrap().size, 0);
        }

        #[test]
        fn symlinks_are_reported_never_followed() {
            let tmp = Tmp::new("symlink");
            std::fs::write(tmp.0.join("target"), "data").unwrap();
            let mut fs = HostFs::new(&tmp.0);

            fs.symlink("/rel", "target").unwrap();
            let a = fs.lstat("/rel").unwrap();
            assert_eq!(a.qid.kind, QT_SYMLINK, "lstat must not follow the link");
            assert_eq!(fs.readlink("/rel").unwrap(), "target");

            // A link pointing out of the export is reported as text — the guest
            // resolves it against its own root — but opening it is refused, so
            // it can never be followed on the host side.
            fs.symlink("/escape", "/etc/passwd").unwrap();
            assert_eq!(fs.readlink("/escape").unwrap(), "/etc/passwd");
            assert_eq!(fs.open("/escape", 0), Err(40 /* ELOOP */));
            assert_eq!(fs.open("/rel", 0), Err(40));
        }

        #[test]
        fn hardlink_and_remove() {
            let tmp = Tmp::new("link");
            let mut fs = HostFs::new(&tmp.0);
            fs.create("/one", O_RDWR, 0o644).unwrap();
            fs.write("/one", 0, b"shared").unwrap();
            fs.hardlink("/one", "/two").unwrap();
            let (a, b) = (fs.lstat("/one").unwrap(), fs.lstat("/two").unwrap());
            assert_eq!(a.qid.path, b.qid.path, "same inode");
            assert_eq!(a.nlink, 2);
            fs.remove("/one", false).unwrap();
            assert_eq!(fs.lstat("/two").unwrap().nlink, 1);
            assert_eq!(fs.remove("/one", false), Err(ENOENT));
            // Removing a directory needs the AT_REMOVEDIR flavour.
            fs.mkdir("/d", 0o755).unwrap();
            assert_eq!(fs.remove("/d", false), Err(EISDIR));
            assert!(fs.remove("/d", true).is_ok());
        }

        #[test]
        fn host_errnos_pass_through_unchanged() {
            let tmp = Tmp::new("errno");
            let mut fs = HostFs::new(&tmp.0);
            assert_eq!(fs.lstat("/missing"), Err(ENOENT));
            assert_eq!(fs.readdir("/missing"), Err(ENOENT));
            fs.create("/f", O_RDWR, 0o644).unwrap();
            assert_eq!(fs.readdir("/f"), Err(ENOTDIR));
            assert_eq!(fs.mkdir("/f", 0o755), Err(EEXIST));
            // A no-op chown succeeds even though a real one would need root.
            let uid = fs.lstat("/f").unwrap().uid;
            assert!(fs.set_owner("/f", Some(uid), None).is_ok());
        }

        #[test]
        fn set_times_are_visible_to_lstat() {
            let tmp = Tmp::new("times");
            let mut fs = HostFs::new(&tmp.0);
            fs.create("/f", O_RDWR, 0o644).unwrap();
            fs.set_times("/f", Some((1_000_000, 0)), Some((2_000_000, 500)))
                .unwrap();
            let a = fs.lstat("/f").unwrap();
            assert_eq!(a.atime.0, 1_000_000);
            assert_eq!(a.mtime, (2_000_000, 500));
        }
    }
}
