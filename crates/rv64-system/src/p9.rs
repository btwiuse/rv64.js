//! 9P2000.L file server — the host half of virtio-9p.
//!
//! virtio-9p is not networking: the guest kernel's `9pnet_virtio` driver puts
//! 9P messages in a virtqueue and *we* are the server, because the host is the
//! side that owns the files. This module speaks the protocol; [`FsBackend`]
//! supplies the files (see `p9fs.rs` for the in-memory and host-directory
//! backends). `virtio.rs` owns the transport and hands us whole request
//! buffers.
//!
//! Reference: TinyEMU's `virtio.c:1646-2650` (`virtio_9p_recv_request`) and
//! `fs_disk.c`. We implement the same message subset, which is what Linux's
//! v9fs actually uses.
//!
//! Two deliberate deviations from TinyEMU, both noted at their call sites:
//! `xattrwalk` returns EOPNOTSUPP rather than TinyEMU's 524 (a kernel-internal
//! ENOTSUPP that no userspace errno maps to), and byte-range locks are granted
//! unconditionally instead of taken on the host.
//!
//! The backend is addressed by *path* rather than by open handle. 9P read and
//! write both carry an explicit offset, so a server needs no per-fid seek
//! state, and path addressing keeps the trait small. The one place statefulness
//! is unavoidable is directory iteration, where `Treaddir` resumes from an
//! opaque offset — so each fid snapshots its directory listing on the
//! `offset == 0` call and indexes into that (see [`Fid::dir`]).

use std::collections::HashMap;

// ---- protocol constants ---------------------------------------------------

// Message ids (T-messages; the reply is always id+1).
const T_STATFS: u8 = 8;
const T_LOPEN: u8 = 12;
const T_LCREATE: u8 = 14;
const T_SYMLINK: u8 = 16;
const T_MKNOD: u8 = 18;
const T_READLINK: u8 = 22;
const T_GETATTR: u8 = 24;
const T_SETATTR: u8 = 26;
const T_XATTRWALK: u8 = 30;
const T_READDIR: u8 = 40;
const T_FSYNC: u8 = 50;
const T_LOCK: u8 = 52;
const T_GETLOCK: u8 = 54;
const T_LINK: u8 = 70;
const T_MKDIR: u8 = 72;
const T_RENAMEAT: u8 = 74;
const T_UNLINKAT: u8 = 76;
const T_VERSION: u8 = 100;
const T_ATTACH: u8 = 104;
const T_FLUSH: u8 = 108;
const T_WALK: u8 = 110;
const T_READ: u8 = 116;
const T_WRITE: u8 = 118;
const T_CLUNK: u8 = 120;
/// Rlerror — the only reply whose id is not `request + 1`.
const R_LERROR: u8 = 7;

/// qid.type bits.
pub const QT_DIR: u8 = 0x80;
pub const QT_SYMLINK: u8 = 0x02;
pub const QT_FILE: u8 = 0x00;

// Host mode bits (S_IFMT family), used to classify backend entries.
pub const S_IFMT: u32 = 0xf000;
pub const S_IFDIR: u32 = 0x4000;
pub const S_IFREG: u32 = 0x8000;
pub const S_IFLNK: u32 = 0xa000;

// `Tlopen`/`Tlcreate` flags we care about.
pub const O_WRONLY: u32 = 0x1;
pub const O_RDWR: u32 = 0x2;
pub const O_TRUNC: u32 = 0x200;
pub const O_DIRECTORY: u32 = 0x1_0000;

// `Tsetattr` valid-mask bits.
const SETATTR_MODE: u32 = 0x001;
const SETATTR_UID: u32 = 0x002;
const SETATTR_GID: u32 = 0x004;
const SETATTR_SIZE: u32 = 0x008;
const SETATTR_ATIME: u32 = 0x010;
const SETATTR_MTIME: u32 = 0x020;
const SETATTR_ATIME_SET: u32 = 0x080;
const SETATTR_MTIME_SET: u32 = 0x100;

/// `Tunlinkat` flag meaning "this is a directory" (Linux `AT_REMOVEDIR`).
const AT_REMOVEDIR: u32 = 0x200;

// Errnos. 9P2000.L carries Linux errno values verbatim, and both sides here
// *are* Linux, so host errnos pass straight through.
pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const EIO: i32 = 5;
pub const EEXIST: i32 = 17;
pub const ENOTDIR: i32 = 20;
pub const EISDIR: i32 = 21;
pub const EINVAL: i32 = 22;
pub const ENOSPC: i32 = 28;
pub const EROFS: i32 = 30;
pub const ENOTEMPTY: i32 = 39;
pub const EPROTO: i32 = 71;
pub const EOPNOTSUPP: i32 = 95;

/// Largest message we will negotiate. Linux ≥5.15 asks for 128 KiB by default
/// and older kernels for 8 KiB; either way one message must fit in a single
/// descriptor chain, so this is bounded by the ring size `virtio.rs` advertises
/// for the 9p device (128 entries × 4 KiB pages leaves ample headroom).
pub const MAX_MSIZE: u32 = 128 * 1024;

// ---- backend interface ----------------------------------------------------

/// Server-side identity of a file, as the client caches it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Qid {
    /// `QT_*`
    pub kind: u8,
    /// Bumped when contents change; 0 means "client must not cache".
    pub version: u32,
    /// Unique file id — the inode number.
    pub path: u64,
}

impl Qid {
    pub fn from_mode(mode: u32, ino: u64) -> Qid {
        Qid {
            kind: qid_kind(mode),
            version: 0,
            path: ino,
        }
    }
}

pub fn qid_kind(mode: u32) -> u8 {
    match mode & S_IFMT {
        S_IFDIR => QT_DIR,
        S_IFLNK => QT_SYMLINK,
        _ => QT_FILE,
    }
}

/// Everything `Tgetattr` can report about a file.
#[derive(Clone, Debug)]
pub struct Attr {
    pub qid: Qid,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u64,
    pub rdev: u64,
    pub size: u64,
    pub blksize: u64,
    pub blocks: u64,
    /// (seconds, nanoseconds)
    pub atime: (u64, u64),
    pub mtime: (u64, u64),
    pub ctime: (u64, u64),
}

impl Attr {
    /// A plausible attr for `mode`/`size`, for backends without real metadata.
    pub fn simple(ino: u64, mode: u32, size: u64) -> Attr {
        Attr {
            qid: Qid::from_mode(mode, ino),
            mode,
            uid: 0,
            gid: 0,
            nlink: if mode & S_IFMT == S_IFDIR { 2 } else { 1 },
            rdev: 0,
            size,
            blksize: 4096,
            blocks: size.div_ceil(512),
            atime: (0, 0),
            mtime: (0, 0),
            ctime: (0, 0),
        }
    }

    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
}

/// One directory entry, as `Treaddir` reports it.
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub ino: u64,
    /// Host mode bits; only the `S_IFMT` part is used (for qid.type/d_type).
    pub mode: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct StatFs {
    pub bsize: u32,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
}

/// The files a [`Server`] exports.
///
/// Paths are export-relative and pre-normalised by the server: the root is
/// `""` and every other path is `/`-prefixed with no `.`/`..` components and
/// no trailing slash (so a backend can concatenate them onto its own root
/// without further checking). All methods return a positive Linux errno on
/// failure.
pub trait FsBackend {
    fn statfs(&mut self) -> StatFs;

    /// Stat without following a trailing symlink (`lstat`).
    fn lstat(&mut self, path: &str) -> Result<Attr, i32>;

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, i32>;

    /// Prepare `path` for I/O with `flags` (`O_*`). Backends that keep open
    /// handles should establish one here; `O_TRUNC` must be honoured.
    fn open(&mut self, path: &str, flags: u32) -> Result<Attr, i32>;

    /// Release any handle `open` established. Called on clunk.
    fn close(&mut self, _path: &str) {}

    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i32>;
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<usize, i32>;

    /// Create a regular file and leave it open (as `Tlcreate` requires).
    fn create(&mut self, path: &str, flags: u32, mode: u32) -> Result<Attr, i32>;
    fn mkdir(&mut self, path: &str, mode: u32) -> Result<Attr, i32>;
    fn symlink(&mut self, path: &str, target: &str) -> Result<Attr, i32>;
    fn mknod(&mut self, path: &str, mode: u32, major: u32, minor: u32) -> Result<Attr, i32>;
    fn readlink(&mut self, path: &str) -> Result<String, i32>;
    fn hardlink(&mut self, existing: &str, new: &str) -> Result<(), i32>;

    /// Remove `path`. `is_dir` comes from the client's `AT_REMOVEDIR` flag.
    fn remove(&mut self, path: &str, is_dir: bool) -> Result<(), i32>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), i32>;

    fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), i32>;
    fn set_owner(&mut self, path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), i32>;
    fn truncate(&mut self, path: &str, size: u64) -> Result<(), i32>;
    fn set_times(
        &mut self,
        path: &str,
        atime: Option<(u64, u64)>,
        mtime: Option<(u64, u64)>,
    ) -> Result<(), i32>;
}

// ---- fids -----------------------------------------------------------------

/// Client-side handle onto one file. The client allocates the numbers; we just
/// remember what each one points at.
struct Fid {
    /// Normalised export-relative path (`""` for the root).
    path: String,
    /// Set by `Tlopen`/`Tlcreate`; `Tread`/`Treaddir` require it.
    opened: bool,
    /// Directory listing snapshot, taken when `Treaddir` asks for offset 0 and
    /// indexed by later calls. Without this, iteration would re-list the
    /// directory per call and entries could shift under the client mid-scan.
    dir: Option<Vec<DirEntry>>,
}

// ---- server ---------------------------------------------------------------

pub struct Server {
    fs: Box<dyn FsBackend>,
    /// virtio config-space mount tag, e.g. `/dev/root` or `host`.
    tag: String,
    msize: u32,
    fids: HashMap<u32, Fid>,
}

impl Server {
    pub fn new(tag: impl Into<String>, fs: Box<dyn FsBackend>) -> Server {
        Server {
            fs,
            tag: tag.into(),
            msize: 8192,
            fids: HashMap::new(),
        }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Handle one T-message, returning the complete R-message (including its
    /// `size`/`id`/`tag` header). Never fails: protocol and backend errors both
    /// come back as `Rlerror`.
    pub fn handle(&mut self, req: &[u8]) -> Vec<u8> {
        let mut r = Rd::new(req);
        // size[4] id[1] tag[2]
        let hdr = (r.u32(), r.u8(), r.u16());
        let (id, tag) = match hdr {
            (Some(_), Some(id), Some(tag)) => (id, tag),
            _ => return reply(R_LERROR, 0, &err_body(EPROTO)),
        };
        match self.dispatch(id, &mut r) {
            Ok(body) => reply(id + 1, tag, &body),
            Err(e) => reply(R_LERROR, tag, &err_body(e)),
        }
    }

    fn dispatch(&mut self, id: u8, r: &mut Rd) -> Result<Vec<u8>, i32> {
        let mut w = Wr::default();
        match id {
            T_VERSION => {
                let msize = r.u32().ok_or(EPROTO)?;
                let version = r.str().ok_or(EPROTO)?;
                // A new session: the client's fids are all gone.
                self.fids.clear();
                self.msize = msize.clamp(4096, MAX_MSIZE);
                w.u32(self.msize);
                // Only 9P2000.L is offered; the client is expected to have
                // asked for it (v9fs does when mounted version=9p2000.L).
                w.str(if version.starts_with("9P2000.L") {
                    "9P2000.L"
                } else {
                    "unknown"
                });
            }
            T_ATTACH => {
                let fid = r.u32().ok_or(EPROTO)?;
                let _afid = r.u32().ok_or(EPROTO)?;
                let _uname = r.str().ok_or(EPROTO)?;
                let _aname = r.str().ok_or(EPROTO)?;
                let _uid = r.u32(); // absent in 9P2000 proper; optional here
                let attr = self.fs.lstat("")?;
                self.set_fid(fid, String::new());
                w.qid(&attr.qid);
            }
            T_WALK => {
                let fid = r.u32().ok_or(EPROTO)?;
                let newfid = r.u32().ok_or(EPROTO)?;
                let nwname = r.u16().ok_or(EPROTO)? as usize;
                let mut path = self.fid_path(fid)?;
                let mut qids = Vec::with_capacity(nwname);
                for _ in 0..nwname {
                    let name = r.str().ok_or(EPROTO)?;
                    let next = match join(&path, &name) {
                        Some(p) => p,
                        None => break,
                    };
                    match self.fs.lstat(&next) {
                        Ok(attr) => {
                            qids.push(attr.qid);
                            path = next;
                        }
                        // A short walk is not an error at this layer: we report
                        // how far we got and the client turns a partial result
                        // into ENOENT itself (p9_client_walk).
                        Err(_) => break,
                    }
                }
                self.set_fid(newfid, path);
                w.u16(qids.len() as u16);
                for q in &qids {
                    w.qid(q);
                }
            }
            T_CLUNK => {
                let fid = r.u32().ok_or(EPROTO)?;
                if let Some(f) = self.fids.remove(&fid) {
                    if f.opened {
                        self.fs.close(&f.path);
                    }
                }
            }
            T_STATFS => {
                let _fid = r.u32().ok_or(EPROTO)?;
                let st = self.fs.statfs();
                w.u32(0); // fs type
                w.u32(st.bsize);
                w.u64(st.blocks);
                w.u64(st.bfree);
                w.u64(st.bavail);
                w.u64(st.files);
                w.u64(st.ffree);
                w.u64(0); // fsid
                w.u32(255); // max filename length
            }
            T_GETATTR => {
                let fid = r.u32().ok_or(EPROTO)?;
                let mask = r.u64().ok_or(EPROTO)?;
                let path = self.fid_path(fid)?;
                let a = self.fs.lstat(&path)?;
                // Echo the requested mask as `valid`: everything v9fs asks for
                // in P9_STATS_BASIC is filled in below. btime/gen/data_version
                // are always zero, and no client requests them.
                w.u64(mask);
                w.qid(&a.qid);
                w.u32(a.mode);
                w.u32(a.uid);
                w.u32(a.gid);
                w.u64(a.nlink);
                w.u64(a.rdev);
                w.u64(a.size);
                w.u64(a.blksize);
                w.u64(a.blocks);
                w.u64(a.atime.0);
                w.u64(a.atime.1);
                w.u64(a.mtime.0);
                w.u64(a.mtime.1);
                w.u64(a.ctime.0);
                w.u64(a.ctime.1);
                w.u64(0); // btime_sec
                w.u64(0); // btime_nsec
                w.u64(0); // gen
                w.u64(0); // data_version
            }
            T_SETATTR => {
                let fid = r.u32().ok_or(EPROTO)?;
                let valid = r.u32().ok_or(EPROTO)?;
                let mode = r.u32().ok_or(EPROTO)?;
                let uid = r.u32().ok_or(EPROTO)?;
                let gid = r.u32().ok_or(EPROTO)?;
                let size = r.u64().ok_or(EPROTO)?;
                let atime = (r.u64().ok_or(EPROTO)?, r.u64().ok_or(EPROTO)?);
                let mtime = (r.u64().ok_or(EPROTO)?, r.u64().ok_or(EPROTO)?);
                let path = self.fid_path(fid)?;
                // Order matters: chown can clear setuid bits, so mode goes last.
                if valid & (SETATTR_UID | SETATTR_GID) != 0 {
                    self.fs.set_owner(
                        &path,
                        (valid & SETATTR_UID != 0).then_some(uid),
                        (valid & SETATTR_GID != 0).then_some(gid),
                    )?;
                }
                if valid & SETATTR_SIZE != 0 {
                    self.fs.truncate(&path, size)?;
                }
                if valid & (SETATTR_ATIME | SETATTR_MTIME) != 0 {
                    // *_SET means "use the value I gave"; without it the client
                    // is asking for "now", which the backend supplies.
                    let a = (valid & SETATTR_ATIME != 0)
                        .then(|| (valid & SETATTR_ATIME_SET != 0).then_some(atime))
                        .flatten();
                    let m = (valid & SETATTR_MTIME != 0)
                        .then(|| (valid & SETATTR_MTIME_SET != 0).then_some(mtime))
                        .flatten();
                    self.fs.set_times(&path, a, m)?;
                }
                if valid & SETATTR_MODE != 0 {
                    self.fs.set_mode(&path, mode)?;
                }
            }
            T_LOPEN => {
                let fid = r.u32().ok_or(EPROTO)?;
                let flags = r.u32().ok_or(EPROTO)?;
                let path = self.fid_path(fid)?;
                let attr = self.fs.open(&path, flags)?;
                let f = self.fids.get_mut(&fid).ok_or(EPROTO)?;
                f.opened = true;
                f.dir = None;
                w.qid(&attr.qid);
                w.u32(self.iounit());
            }
            T_LCREATE => {
                let fid = r.u32().ok_or(EPROTO)?;
                let name = r.str().ok_or(EPROTO)?;
                let flags = r.u32().ok_or(EPROTO)?;
                let mode = r.u32().ok_or(EPROTO)?;
                let _gid = r.u32().ok_or(EPROTO)?;
                let dir = self.fid_path(fid)?;
                let path = join(&dir, &name).ok_or(EINVAL)?;
                let attr = self.fs.create(&path, flags, mode)?;
                // The fid moves from the directory to the new file.
                let f = self.fids.get_mut(&fid).ok_or(EPROTO)?;
                f.path = path;
                f.opened = true;
                f.dir = None;
                w.qid(&attr.qid);
                w.u32(self.iounit());
            }
            T_READ => {
                let fid = r.u32().ok_or(EPROTO)?;
                let offset = r.u64().ok_or(EPROTO)?;
                let count = r.u32().ok_or(EPROTO)?;
                let (path, opened) = self.fid_state(fid)?;
                if !opened {
                    return Err(EPROTO);
                }
                // Reply is count[4] + data, inside msize (11 = header + count).
                let count = count.min(self.msize.saturating_sub(11)) as usize;
                let mut buf = vec![0u8; count];
                let n = self.fs.read(&path, offset, &mut buf)?;
                w.u32(n as u32);
                w.bytes(&buf[..n]);
            }
            T_WRITE => {
                let fid = r.u32().ok_or(EPROTO)?;
                let offset = r.u64().ok_or(EPROTO)?;
                let count = r.u32().ok_or(EPROTO)? as usize;
                let data = r.take(count).ok_or(EPROTO)?.to_vec();
                let (path, opened) = self.fid_state(fid)?;
                if !opened {
                    return Err(EPROTO);
                }
                let n = self.fs.write(&path, offset, &data)?;
                w.u32(n as u32);
            }
            T_READDIR => {
                let fid = r.u32().ok_or(EPROTO)?;
                let offset = r.u64().ok_or(EPROTO)?;
                let count = r.u32().ok_or(EPROTO)?;
                let body = self.readdir(fid, offset, count)?;
                w.u32(body.len() as u32);
                w.bytes(&body);
            }
            T_MKDIR => {
                let dfid = r.u32().ok_or(EPROTO)?;
                let name = r.str().ok_or(EPROTO)?;
                let mode = r.u32().ok_or(EPROTO)?;
                let _gid = r.u32().ok_or(EPROTO)?;
                let path = self.child(dfid, &name)?;
                let attr = self.fs.mkdir(&path, mode)?;
                w.qid(&attr.qid);
            }
            T_UNLINKAT => {
                let dfid = r.u32().ok_or(EPROTO)?;
                let name = r.str().ok_or(EPROTO)?;
                let flags = r.u32().ok_or(EPROTO)?;
                let path = self.child(dfid, &name)?;
                self.fs.remove(&path, flags & AT_REMOVEDIR != 0)?;
            }
            T_RENAMEAT => {
                let olddirfid = r.u32().ok_or(EPROTO)?;
                let oldname = r.str().ok_or(EPROTO)?;
                let newdirfid = r.u32().ok_or(EPROTO)?;
                let newname = r.str().ok_or(EPROTO)?;
                let from = self.child(olddirfid, &oldname)?;
                let to = self.child(newdirfid, &newname)?;
                self.fs.rename(&from, &to)?;
            }
            T_SYMLINK => {
                let dfid = r.u32().ok_or(EPROTO)?;
                let name = r.str().ok_or(EPROTO)?;
                let target = r.str().ok_or(EPROTO)?;
                let _gid = r.u32().ok_or(EPROTO)?;
                let path = self.child(dfid, &name)?;
                let attr = self.fs.symlink(&path, &target)?;
                w.qid(&attr.qid);
            }
            T_READLINK => {
                let fid = r.u32().ok_or(EPROTO)?;
                let path = self.fid_path(fid)?;
                let target = self.fs.readlink(&path)?;
                w.str(&target);
            }
            T_LINK => {
                let dfid = r.u32().ok_or(EPROTO)?;
                let fid = r.u32().ok_or(EPROTO)?;
                let name = r.str().ok_or(EPROTO)?;
                let existing = self.fid_path(fid)?;
                let new = self.child(dfid, &name)?;
                self.fs.hardlink(&existing, &new)?;
            }
            T_MKNOD => {
                let dfid = r.u32().ok_or(EPROTO)?;
                let name = r.str().ok_or(EPROTO)?;
                let mode = r.u32().ok_or(EPROTO)?;
                let major = r.u32().ok_or(EPROTO)?;
                let minor = r.u32().ok_or(EPROTO)?;
                let _gid = r.u32().ok_or(EPROTO)?;
                let path = self.child(dfid, &name)?;
                let attr = self.fs.mknod(&path, mode, major, minor)?;
                w.qid(&attr.qid);
            }
            // Writes reach the backend synchronously, so there is nothing to
            // flush; and a request is fully serviced before we publish its
            // reply, so there is never an in-flight request to cancel.
            T_FSYNC | T_FLUSH => {}
            T_XATTRWALK => {
                // No xattr support. EOPNOTSUPP is what v9fs's ACL probe expects
                // (TinyEMU answers 524 = the kernel-internal ENOTSUPP, which
                // maps to no userspace errno).
                return Err(EOPNOTSUPP);
            }
            T_LOCK => {
                let _fid = r.u32().ok_or(EPROTO)?;
                // Granted unconditionally: one guest owns the export, and
                // reflecting its locks onto the host would only matter if the
                // host contended for the same files during the run.
                w.u8(0); // P9_LOCK_SUCCESS
            }
            T_GETLOCK => {
                let _fid = r.u32().ok_or(EPROTO)?;
                let _kind = r.u8().ok_or(EPROTO)?;
                let start = r.u64().ok_or(EPROTO)?;
                let length = r.u64().ok_or(EPROTO)?;
                let proc_id = r.u32().ok_or(EPROTO)?;
                let client_id = r.str().ok_or(EPROTO)?;
                w.u8(2); // P9_LOCK_TYPE_UNLCK: nobody holds it
                w.u64(start);
                w.u64(length);
                w.u32(proc_id);
                w.str(&client_id);
            }
            _ => return Err(EOPNOTSUPP),
        }
        Ok(w.0)
    }

    /// `Treaddir` body: `qid[13] offset[8] type[1] name[s]` per entry, stopping
    /// before `count` is exceeded.
    fn readdir(&mut self, fid: u32, offset: u64, count: u32) -> Result<Vec<u8>, i32> {
        let f = self.fids.get(&fid).ok_or(EPROTO)?;
        if !f.opened {
            return Err(EPROTO);
        }
        let path = f.path.clone();
        if offset == 0 || self.fids[&fid].dir.is_none() {
            // "." and ".." are not in a backend listing but getdents callers
            // expect them, so synthesise both ahead of the real entries.
            let mut entries = Vec::new();
            let self_attr = self.fs.lstat(&path)?;
            entries.push(DirEntry {
                name: ".".into(),
                ino: self_attr.qid.path,
                mode: self_attr.mode,
            });
            let parent_path = parent(&path);
            let parent_attr = self.fs.lstat(&parent_path)?;
            entries.push(DirEntry {
                name: "..".into(),
                ino: parent_attr.qid.path,
                mode: parent_attr.mode,
            });
            entries.extend(self.fs.readdir(&path)?);
            self.fids.get_mut(&fid).ok_or(EPROTO)?.dir = Some(entries);
        }
        let entries = self.fids[&fid].dir.as_ref().ok_or(EIO)?;

        let mut out = Vec::new();
        let count = count.min(self.msize.saturating_sub(11)) as usize;
        // The offset a client sends back is the one we stamped on the last
        // entry it consumed, and we stamp entry i with i+1 — so it doubles as
        // the resume index into the snapshot.
        for (i, e) in entries.iter().enumerate().skip(offset as usize) {
            let size = 13 + 8 + 1 + 2 + e.name.len();
            if out.len() + size > count {
                break;
            }
            let mut w = Wr(out);
            w.qid(&Qid::from_mode(e.mode, e.ino));
            w.u64(i as u64 + 1);
            w.u8(((e.mode & S_IFMT) >> 12) as u8); // d_type
            w.str(&e.name);
            out = w.0;
        }
        Ok(out)
    }

    /// Largest payload the client may put in one read/write, per `Tlopen`.
    fn iounit(&self) -> u32 {
        // Leave room for the reply header plus the largest fixed field set.
        self.msize.saturating_sub(24)
    }

    fn set_fid(&mut self, fid: u32, path: String) {
        if let Some(old) = self.fids.remove(&fid) {
            if old.opened {
                self.fs.close(&old.path);
            }
        }
        self.fids.insert(
            fid,
            Fid {
                path,
                opened: false,
                dir: None,
            },
        );
    }

    fn fid_path(&self, fid: u32) -> Result<String, i32> {
        self.fids.get(&fid).map(|f| f.path.clone()).ok_or(EPROTO)
    }

    fn fid_state(&self, fid: u32) -> Result<(String, bool), i32> {
        self.fids
            .get(&fid)
            .map(|f| (f.path.clone(), f.opened))
            .ok_or(EPROTO)
    }

    /// Path of `name` inside the directory `dfid` refers to.
    fn child(&self, dfid: u32, name: &str) -> Result<String, i32> {
        let dir = self.fid_path(dfid)?;
        join(&dir, name).ok_or(EINVAL)
    }
}

// ---- path handling --------------------------------------------------------

/// Append one path component, returning `None` for anything a server must not
/// resolve. `.` and `..` are handled here rather than rejected — a client may
/// legitimately walk them, and collapsing `..` against the export root is what
/// keeps a guest from escaping it.
fn join(base: &str, name: &str) -> Option<String> {
    match name {
        "" => None,
        "." => Some(base.to_string()),
        ".." => Some(parent(base)),
        _ if name.contains('/') || name.contains('\0') => None,
        _ => Some(format!("{base}/{name}")),
    }
}

/// Containing directory of `path`; the root is its own parent.
fn parent(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

// ---- message framing -----------------------------------------------------

fn reply(id: u8, tag: u16, body: &[u8]) -> Vec<u8> {
    let len = body.len() + 7;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.push(id);
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(body);
    out
}

fn err_body(errno: i32) -> Vec<u8> {
    (errno as u32).to_le_bytes().to_vec()
}

/// Little-endian reader over a request body.
struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Rd<'a> {
        Rd { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes(s.try_into().unwrap()))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u16()? as usize;
        // Guest filenames are bytes, not guaranteed UTF-8; lossy conversion
        // keeps us from failing a whole request over one odd name.
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

/// Little-endian writer for a reply body.
#[derive(Default)]
struct Wr(Vec<u8>);

impl Wr {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn qid(&mut self, q: &Qid) {
        self.u8(q.kind);
        self.u32(q.version);
        self.u64(q.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_keeps_the_guest_inside_the_export() {
        // `..` collapses instead of escaping, and the root is its own parent.
        assert_eq!(join("", "usr").unwrap(), "/usr");
        assert_eq!(join("/usr", "bin").unwrap(), "/usr/bin");
        assert_eq!(join("/usr/bin", "..").unwrap(), "/usr");
        assert_eq!(join("/usr", "..").unwrap(), "");
        assert_eq!(join("", "..").unwrap(), "");
        assert_eq!(join("/usr", ".").unwrap(), "/usr");
        // A component may never contain a separator or a NUL.
        assert!(join("", "../etc").is_none());
        assert!(join("", "a/b").is_none());
        assert!(join("", "a\0b").is_none());
        assert!(join("", "").is_none());
    }
}
