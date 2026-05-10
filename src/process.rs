//! Process-group helpers for bounded local command execution.

use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[cfg(unix)]
pub fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
pub fn configure_process_group(_command: &mut Command) {}

pub fn terminate_process_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as i32;
        signal_process_group(pgid, SIGTERM);
        if wait_for_exit(child, Duration::from_millis(250)) {
            return;
        }
        signal_process_group(pgid, SIGKILL);
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => return false,
        }
    }
}

#[cfg(unix)]
const SIGTERM: i32 = 15;
#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
fn signal_process_group(pgid: i32, signal: i32) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    // Negative pid targets the process group. The child is started as its own
    // process-group leader by `configure_process_group`, so descendants spawned
    // by shells are terminated with the timed-out command.
    unsafe {
        let _ = kill(-pgid, signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    #[cfg(unix)]
    fn terminate_process_group_kills_shell_descendants() {
        let marker =
            std::env::temp_dir().join(format!("kubectui-process-group-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);

        let script = format!(
            "trap '' HUP; (trap '' HUP; sleep 2; echo leaked > '{}') & wait",
            marker.display()
        );
        let mut command = Command::new("sh");
        command
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = command.spawn().expect("spawn shell");

        std::thread::sleep(Duration::from_millis(100));
        terminate_process_group(&mut child);
        std::thread::sleep(Duration::from_secs(3));

        assert!(!marker.exists(), "timeout cleanup left descendant alive");
        let _ = std::fs::remove_file(marker);
    }
}
