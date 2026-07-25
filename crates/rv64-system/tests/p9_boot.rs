//! End-to-end virtio-9p: boot Linux, mount a host directory in the guest, and
//! move files across in both directions.
//!
//! The guest is the TinyEMU image set (`web/get-images.sh`), whose kernel has
//! v9fs and `9pnet_virtio` built in — unlike the stock nixpkgs riscv64 kernel,
//! which ships them as modules. Skips (passes) when the images are absent.
//!
//! Slow in a debug build; the suite runs `cargo test --release`.

use rv64_system::{p9, p9fs, BootImages, Machine};
use std::path::PathBuf;

fn images() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/images");
    let read = |f: &str| std::fs::read(dir.join(f)).ok();
    Some((
        read("bbl64.bin")?,
        read("kernel-riscv64.bin")?,
        read("root-riscv64.bin")?,
    ))
}

/// Guest console driver: runs the machine until `needle` appears, returning
/// everything seen so far.
struct Console {
    m: Machine,
    out: String,
}

impl Console {
    fn wait_for(&mut self, needle: &str, slices: usize) -> bool {
        for _ in 0..slices {
            self.m.run_slice(5_000_000);
            let chunk = self.m.console_output();
            if !chunk.is_empty() {
                self.out.push_str(&String::from_utf8_lossy(&chunk));
            }
            if self.out.contains(needle) {
                return true;
            }
            if self.m.power_off {
                break;
            }
        }
        false
    }

    fn send(&mut self, line: &str) {
        self.m.console_input(line.as_bytes());
        self.m.console_input(b"\n");
    }

    /// Run `cmd` and wait for `marker` in its *output*.
    ///
    /// The guest echoes what we type, so a marker that appears verbatim in the
    /// command would match the echo and pass without the command running. Any
    /// `echo OK_x` marker must therefore be written so the typed form differs
    /// from the printed one — `OK_'x'` does that.
    fn run(&mut self, cmd: &str, marker: &str, slices: usize) -> bool {
        assert!(
            !cmd.contains(marker),
            "marker {marker:?} appears in the command, so it would match the echo"
        );
        self.send(cmd);
        self.wait_for(marker, slices)
    }
}

#[test]
fn guest_mounts_and_uses_a_host_directory() {
    let Some((bios, kernel, disk)) = images() else {
        eprintln!("SKIP p9_boot (run web/get-images.sh)");
        return;
    };

    // A host directory with known contents, in a location the guest cannot
    // reach except through the export.
    let share = std::env::temp_dir().join(format!("rv64-9p-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&share);
    std::fs::create_dir_all(share.join("subdir")).expect("create share");
    std::fs::write(share.join("hello.txt"), "9p-works-42\n").expect("write host file");
    std::fs::write(share.join("subdir/deep.txt"), "nested-ok\n").expect("write host file");

    let m = Machine::new(
        128,
        BootImages {
            bios: &bios,
            kernel: Some(&kernel),
            cmdline: "console=hvc0 root=/dev/vda rw",
            disk: Some(disk),
            fs: Some(p9::Server::new(
                "hostshare",
                Box::new(p9fs::HostFs::new(&share)),
            )),
        },
    );
    let mut c = Console {
        m,
        out: String::new(),
    };

    assert!(
        c.wait_for("~ #", 4000),
        "guest never reached a shell:\n{}",
        tail(&c.out)
    );

    // Mount the export. `version=9p2000.L` is what this server speaks.
    assert!(
        c.run(
            "mkdir -p /9p && mount -t 9p -o trans=virtio,version=9p2000.L hostshare /9p && echo MOUNT_'O'K",
            "MOUNT_OK",
            2000,
        ),
        "mount failed:\n{}",
        tail(&c.out)
    );

    // Read a host file through the mount (walk + getattr + lopen + read).
    assert!(
        c.run("cat /9p/hello.txt", "9p-works-42", 2000),
        "reading a host file failed:\n{}",
        tail(&c.out)
    );
    // And one a directory deeper.
    assert!(
        c.run("cat /9p/subdir/deep.txt", "nested-ok", 2000),
        "reading a nested host file failed:\n{}",
        tail(&c.out)
    );

    // Directory listing (Treaddir), including the synthesised dot entries.
    assert!(
        c.run("ls -a /9p | tr '\\n' ' '", "hello.txt", 2000),
        "listing the export failed:\n{}",
        tail(&c.out)
    );

    // Write from the guest and check it lands on the host filesystem
    // (lcreate + write + setattr).
    assert!(
        c.run(
            "echo from-the-guest > /9p/guest.txt && sync && echo WRITE_'O'K",
            "WRITE_OK",
            2000,
        ),
        "writing through the mount failed:\n{}",
        tail(&c.out)
    );
    let landed = std::fs::read_to_string(share.join("guest.txt")).expect("guest.txt on host");
    assert_eq!(landed, "from-the-guest\n");

    // Appending must extend the file rather than replace it (write at offset).
    assert!(
        c.run(
            "echo second-line >> /9p/guest.txt && echo APPEND_'O'K",
            "APPEND_OK",
            2000,
        ),
        "appending through the mount failed:\n{}",
        tail(&c.out)
    );
    assert_eq!(
        std::fs::read_to_string(share.join("guest.txt")).expect("guest.txt"),
        "from-the-guest\nsecond-line\n"
    );

    // Guest-side mkdir + copy + unlink, all visible on the host.
    //
    // Deliberately not `mv`: rename(2) is broken in this 2017 guest userland
    // for *every* filesystem (`mv /tmp/x /tmp/y` fails with ENOSYS too — its
    // libc calls a rename syscall riscv64 never had), so it would test the
    // guest, not us. Trenameat is covered against a real host directory in
    // the HostFs backend tests instead.
    assert!(
        c.run(
            "mkdir /9p/made && cp /9p/guest.txt /9p/made/copied.txt && echo COPY_'O'K",
            "COPY_OK",
            2000,
        ),
        "mkdir/copy through the mount failed:\n{}",
        tail(&c.out)
    );
    assert!(share.join("made").is_dir(), "guest mkdir reached the host");
    assert_eq!(
        std::fs::read_to_string(share.join("made/copied.txt")).unwrap_or_else(|e| panic!(
            "copied.txt not on the host ({e}); share now holds {:?}\n{}",
            listing(&share),
            tail(&c.out)
        )),
        "from-the-guest\nsecond-line\n"
    );

    assert!(
        c.run(
            "rm /9p/made/copied.txt && rmdir /9p/made && echo RM_'O'K",
            "RM_OK",
            2000,
        ),
        "unlink/rmdir through the mount failed:\n{}",
        tail(&c.out)
    );
    assert!(!share.join("made").exists(), "rmdir reached the host");

    // A host-side change must be visible to the guest afterwards (no stale
    // caching on our side of the wire).
    std::fs::write(share.join("late.txt"), "host-wrote-later\n").expect("write late file");
    assert!(
        c.run("cat /9p/late.txt", "host-wrote-later", 2000),
        "a file created on the host after mount was not visible:\n{}",
        tail(&c.out)
    );

    let _ = std::fs::remove_dir_all(&share);
}

/// Recursive listing of the export, for assertion messages.
fn listing(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        out.push(p.strip_prefix(dir).unwrap().display().to_string());
        if p.is_dir() {
            for sub in listing(&p) {
                out.push(format!("{}/{sub}", p.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    out.sort();
    out
}

/// Last few lines of guest output, for assertion messages.
fn tail(out: &str) -> String {
    let lines: Vec<&str> = out.lines().collect();
    lines[lines.len().saturating_sub(25)..].join("\n")
}
