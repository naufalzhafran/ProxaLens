//! Watchdog for layer 3 of the system proxy crash safety (see docs/PLAN.md).
//!
//! The app re-executes its own binary as `<exe> --guard <pid> <marker>` in a
//! new process group, so the watchdog survives a crash, `kill -9`, or a
//! Ctrl-C in the dev terminal. It waits for the app's PID to exit using
//! kqueue and, if the app left its marker behind, restores the proxy.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::sysproxy::{self, NetworkSetup, RealNetworkSetup};

pub const GUARD_FLAG: &str = "--guard";

/// Starts the watchdog for the current process.
pub fn spawn(marker_path: &Path) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .arg(GUARD_FLAG)
        .arg(std::process::id().to_string())
        .arg(marker_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
}

/// If the process was launched as a watchdog, runs it and returns `true`.
pub fn run_if_requested() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(GUARD_FLAG) {
        return false;
    }
    let (Some(pid), Some(marker)) = (args.get(2).and_then(|p| p.parse().ok()), args.get(3)) else {
        eprintln!("usage: {GUARD_FLAG} <pid> <marker-path>");
        std::process::exit(2);
    };
    run(pid, PathBuf::from(marker), &RealNetworkSetup);
    true
}

pub fn run(parent: u32, marker_path: PathBuf, ns: &dyn NetworkSetup) {
    unsafe {
        // Ignore terminal interrupts; the app stops us with SIGTERM.
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    wait_for_exit(parent);
    restore_if_left_behind(parent, &marker_path, ns);
}

/// Restores only when the marker still belongs to the dead parent. A clean
/// exit removes the marker first, so this is then a no-op.
pub fn restore_if_left_behind(parent: u32, marker_path: &Path, ns: &dyn NetworkSetup) -> bool {
    match sysproxy::read_marker(marker_path) {
        Ok(Some(m)) if m.pid == parent => sysproxy::restore(ns, marker_path, &m).is_ok(),
        _ => false,
    }
}

/// Blocks until `pid` exits.
fn wait_for_exit(pid: u32) {
    unsafe {
        let kq = libc::kqueue();
        if kq >= 0 {
            let mut ev: libc::kevent = std::mem::zeroed();
            ev.ident = pid as libc::uintptr_t;
            ev.filter = libc::EVFILT_PROC;
            ev.flags = libc::EV_ADD | libc::EV_ONESHOT;
            ev.fflags = libc::NOTE_EXIT;
            let mut out: libc::kevent = std::mem::zeroed();
            // Register and wait in one call. Fails with ESRCH if the
            // process is already gone, which is what we wait for anyway.
            loop {
                let n = libc::kevent(kq, &ev, 1, &mut out, 1, std::ptr::null());
                if n > 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                    break;
                }
            }
            libc::close(kq);
        }
    }
    // Belt and braces in case kqueue was unavailable or woke early.
    while sysproxy::pid_alive(pid) {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::sysproxy::tests::FakeNs;
    use crate::engine::sysproxy::{enable, write_marker};

    #[test]
    fn waits_for_child_exit_then_restores() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        let mut m = enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap();

        // Stand-in for the app: a short-lived process we then kill -9.
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        m.pid = child.id();
        write_marker(&marker, &m).unwrap();

        let pid = child.id();
        let killer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            child.wait().unwrap();
        });
        wait_for_exit(pid);
        killer.join().unwrap();

        assert!(restore_if_left_behind(pid, &marker, &ns));
        assert!(!marker.exists());
        assert!(!ns.get("Wi-Fi", "http").enabled);
    }

    #[test]
    fn ignores_marker_owned_by_another_process() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("m.json");
        let ns = FakeNs::new(&["Wi-Fi"]);
        enable(&ns, &marker, "127.0.0.1", 9090, 1.0).unwrap(); // pid = ours
        assert!(!restore_if_left_behind(1, &marker, &ns));
        assert!(marker.exists());
    }
}
