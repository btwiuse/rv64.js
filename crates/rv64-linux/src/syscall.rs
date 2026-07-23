//! Linux syscall emulation for riscv64 (the "generic" Linux syscall ABI).
//!
//! ABI: number in a7 (x17), args in a0..a5 (x10..x15), result in a0.
//! Unknown syscalls return -ENOSYS — enough for musl static binaries; the
//! set grows as guests demand more.

use crate::{Host, Machine};

// riscv64 uses the asm-generic syscall table.
const SYS_GETCWD: u64 = 17;
const SYS_FCNTL: u64 = 25;
const SYS_IOCTL: u64 = 29;
const SYS_UNLINKAT: u64 = 35;
const SYS_FACCESSAT: u64 = 48;
const SYS_OPENAT: u64 = 56;
const SYS_CLOSE: u64 = 57;
const SYS_LSEEK: u64 = 62;
const SYS_READ: u64 = 63;
const SYS_WRITE: u64 = 64;
const SYS_READV: u64 = 65;
const SYS_WRITEV: u64 = 66;
const SYS_PPOLL: u64 = 73;
const SYS_READLINKAT: u64 = 78;
const SYS_NEWFSTATAT: u64 = 79;
const SYS_FSTAT: u64 = 80;
const SYS_EXIT: u64 = 93;
const SYS_EXIT_GROUP: u64 = 94;
const SYS_SET_TID_ADDRESS: u64 = 96;
const SYS_FUTEX: u64 = 98;
const SYS_SET_ROBUST_LIST: u64 = 99;
const SYS_CLOCK_GETTIME: u64 = 113;
const SYS_SCHED_GETAFFINITY: u64 = 123;
const SYS_SIGALTSTACK: u64 = 132;
const SYS_RT_SIGACTION: u64 = 134;
const SYS_RT_SIGPROCMASK: u64 = 135;
const SYS_UNAME: u64 = 160;
const SYS_GETRUSAGE: u64 = 165;
const SYS_GETTIMEOFDAY: u64 = 169;
const SYS_GETPID: u64 = 172;
const SYS_GETPPID: u64 = 173;
const SYS_GETUID: u64 = 174;
const SYS_GETEUID: u64 = 175;
const SYS_GETGID: u64 = 176;
const SYS_GETEGID: u64 = 177;
const SYS_GETTID: u64 = 178;
const SYS_BRK: u64 = 214;
const SYS_MUNMAP: u64 = 215;
const SYS_MREMAP: u64 = 216;
const SYS_MMAP: u64 = 222;
const SYS_MPROTECT: u64 = 226;
const SYS_MADVISE: u64 = 233;
const SYS_PRLIMIT64: u64 = 261;
const SYS_GETRANDOM: u64 = 278;
const SYS_RSEQ: u64 = 293;
const SYS_RISCV_FLUSH_ICACHE: u64 = 259;

const ENOSYS: i64 = -38;
const EBADF: i64 = -9;
const EINVAL: i64 = -22;
const ENOENT: i64 = -2;
const ENOMEM: i64 = -12;

const PAGE: u64 = 4096;

/// Service the syscall at the current ecall stop. Returns Some(exit_code)
/// when the program terminated.
pub fn handle(m: &mut Machine, host: &mut dyn Host) -> Option<i32> {
    let nr = m.cpu.x[17];
    let a = [
        m.cpu.x[10],
        m.cpu.x[11],
        m.cpu.x[12],
        m.cpu.x[13],
        m.cpu.x[14],
        m.cpu.x[15],
    ];

    let ret: i64 = match nr {
        SYS_EXIT | SYS_EXIT_GROUP => return Some(a[0] as i32),

        SYS_WRITE => sys_write(m, host, a[0] as i32, a[1], a[2]),
        SYS_WRITEV => {
            let (fd, iov, iovcnt) = (a[0] as i32, a[1], a[2] as usize);
            let mut total: i64 = 0;
            for i in 0..iovcnt.min(64) {
                let base = read_u64(m, iov + (i as u64) * 16);
                let len = read_u64(m, iov + (i as u64) * 16 + 8);
                let n = sys_write(m, host, fd, base, len);
                if n < 0 {
                    total = n;
                    break;
                }
                total += n;
                if (n as u64) < len {
                    break;
                }
            }
            total
        }
        SYS_READ | SYS_READV | SYS_PPOLL => 0, // EOF / nothing ready (no stdin yet)

        SYS_BRK => {
            let req = a[0];
            if req >= m.brk_start && req < m.mmap_top {
                m.brk = req;
            }
            m.brk as i64
        }
        SYS_MMAP => {
            // Anonymous mappings only: bump downward from mmap_top.
            let len = (a[1] + PAGE - 1) & !(PAGE - 1);
            let addr = a[0];
            if len == 0 {
                EINVAL
            } else if addr != 0 {
                // Fixed-ish mapping request: accept it if in range (memory
                // is flat and always "mapped"); MAP_FIXED just works.
                addr as i64
            } else if m.mmap_top - len <= m.brk {
                ENOMEM
            } else {
                m.mmap_top -= len;
                // Fresh anonymous memory must be zeroed (may reuse freed space).
                let (s, e) = (m.mmap_top as usize, (m.mmap_top + len) as usize);
                m.mem[s..e].fill(0);
                m.mmap_top as i64
            }
        }
        SYS_MUNMAP | SYS_MPROTECT | SYS_MADVISE | SYS_RISCV_FLUSH_ICACHE => 0,
        SYS_MREMAP => ENOMEM,

        SYS_CLOCK_GETTIME => {
            let ns = host.clock_ns();
            write_u64(m, a[1], ns / 1_000_000_000);
            write_u64(m, a[1] + 8, ns % 1_000_000_000);
            0
        }
        SYS_GETTIMEOFDAY => {
            let ns = host.clock_ns();
            write_u64(m, a[0], ns / 1_000_000_000);
            write_u64(m, a[0] + 8, (ns % 1_000_000_000) / 1000);
            0
        }
        SYS_GETRANDOM => {
            let (buf, len) = (a[0] as usize, (a[1] as usize).min(256));
            let mut tmp = vec![0u8; len];
            host.random(&mut tmp);
            m.mem[buf..buf + len].copy_from_slice(&tmp);
            len as i64
        }

        SYS_UNAME => {
            // struct utsname: 6 fields × 65 bytes
            let base = a[0] as usize;
            m.mem[base..base + 65 * 6].fill(0);
            let put = |mem: &mut [u8], i: usize, s: &str| {
                mem[base + i * 65..base + i * 65 + s.len()].copy_from_slice(s.as_bytes());
            };
            put(&mut m.mem, 0, "Linux");
            put(&mut m.mem, 1, "rv64js");
            put(&mut m.mem, 2, "6.1.0");
            put(&mut m.mem, 3, "#1 rv64.js");
            put(&mut m.mem, 4, "riscv64");
            0
        }
        SYS_GETCWD => {
            let s = b"/\0";
            let base = a[0] as usize;
            if a[1] < 2 {
                EINVAL
            } else {
                m.mem[base..base + 2].copy_from_slice(s);
                a[0] as i64
            }
        }

        SYS_FSTAT | SYS_NEWFSTATAT => {
            // Zeroed struct stat with st_mode = character device (tty-ish)
            // for fds 0-2; that satisfies musl's isatty probing.
            let statbuf = if nr == SYS_FSTAT { a[1] } else { a[2] };
            let base = statbuf as usize;
            m.mem[base..base + 128].fill(0);
            // offsetof(st_mode) = 16 in riscv64 struct stat
            let mode: u32 = 0o020620; // S_IFCHR | 0620
            m.mem[base + 16..base + 20].copy_from_slice(&mode.to_le_bytes());
            0
        }
        SYS_IOCTL => 0, // pretend TIOCGWINSZ etc. succeed
        SYS_FCNTL => 0,
        SYS_OPENAT | SYS_FACCESSAT | SYS_UNLINKAT | SYS_READLINKAT => ENOENT, // no fs yet
        SYS_CLOSE | SYS_LSEEK => {
            if (a[0] as i32) <= 2 {
                0
            } else {
                EBADF
            }
        }

        SYS_SET_TID_ADDRESS | SYS_GETTID | SYS_GETPID => 42,
        SYS_GETPPID => 1,
        SYS_GETUID | SYS_GETEUID | SYS_GETGID | SYS_GETEGID => 1000,
        SYS_SET_ROBUST_LIST | SYS_RT_SIGACTION | SYS_RT_SIGPROCMASK | SYS_SIGALTSTACK
        | SYS_PRLIMIT64 | SYS_GETRUSAGE | SYS_RSEQ => 0,
        SYS_FUTEX => 0, // single-threaded: wake 0 waiters / wait returns
        SYS_SCHED_GETAFFINITY => {
            if a[1] >= 8 {
                write_u64(m, a[2], 1); // one cpu
                8
            } else {
                EINVAL
            }
        }

        _ => ENOSYS,
    };

    m.cpu.x[10] = ret as u64;
    None
}

fn sys_write(m: &mut Machine, host: &mut dyn Host, fd: i32, buf: u64, len: u64) -> i64 {
    if fd < 1 || fd > 2 {
        return EBADF;
    }
    let (s, e) = (buf as usize, (buf + len) as usize);
    if e > m.mem.len() {
        return EINVAL;
    }
    host.write_out(fd, &m.mem[s..e]);
    len as i64
}

fn read_u64(m: &Machine, addr: u64) -> u64 {
    u64::from_le_bytes(m.mem[addr as usize..addr as usize + 8].try_into().unwrap())
}

fn write_u64(m: &mut Machine, addr: u64, v: u64) {
    m.mem[addr as usize..addr as usize + 8].copy_from_slice(&v.to_le_bytes());
}
