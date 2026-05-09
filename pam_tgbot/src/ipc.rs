use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

pub const IPC_DIR: &str = "/run/tgbot/pam";

/// Generate a 32-char hex ID from /dev/urandom (no external crate needed).
pub fn gen_id() -> Option<String> {
    use std::io::Read;
    let mut buf = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut buf)
        .ok()?;
    Some(buf.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Create /run/tgbot/pam/<id> with content "pending", mode 0660.
/// Mode 0660 = tgbot group can write the response back.
pub fn create_pending(id: &str) -> io::Result<PathBuf> {
    let path = Path::new(IPC_DIR).join(id);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o660)
        .open(&path)?;
    f.write_all(b"pending")?;
    // Force 0660 explicitly — OpenOptions::mode() is applied before umask,
    // and sshd typically runs with umask 0022/0027 which strips group-write,
    // preventing the bot from writing the approve/deny response.
    f.set_permissions(std::fs::Permissions::from_mode(0o660))?;
    Ok(path)
}

/// Poll the IPC file every 500ms until content changes from "pending".
/// Returns Some(true)=approved, Some(false)=denied, None=timeout.
/// Always removes the file before returning.
pub fn poll_response(path: &PathBuf, timeout_secs: u64) -> Option<bool> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        thread::sleep(Duration::from_millis(500));
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let trimmed = content.trim().to_ascii_lowercase();
                if trimmed == "pending" {
                    // Not yet answered — keep waiting
                } else {
                    let _ = std::fs::remove_file(path);
                    return Some(trimmed == "approved");
                }
            }
            Err(_) => {
                // File gone — treat as denied
                return Some(false);
            }
        }
        if Instant::now() >= deadline {
            let _ = std::fs::remove_file(path);
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_gen_id_is_32_hex_chars() {
        let id = gen_id().expect("gen_id failed");
        assert_eq!(id.len(), 32, "ID should be 32 hex chars");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "ID should be hex"
        );
    }

    #[test]
    fn test_gen_id_unique() {
        let a = gen_id().unwrap();
        let b = gen_id().unwrap();
        assert_ne!(a, b, "Two generated IDs should differ");
    }

    #[test]
    fn test_poll_approved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ipc_approve");
        fs::write(&path, b"pending").unwrap();

        let p = path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            fs::write(&p, b"approved").unwrap();
        });

        assert_eq!(poll_response(&path, 5), Some(true));
    }

    #[test]
    fn test_poll_denied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ipc_deny");
        fs::write(&path, b"pending").unwrap();

        let p = path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            fs::write(&p, b"denied").unwrap();
        });

        assert_eq!(poll_response(&path, 5), Some(false));
    }

    #[test]
    fn test_poll_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ipc_timeout");
        fs::write(&path, b"pending").unwrap();

        // 1-second timeout, nobody writes anything
        assert_eq!(poll_response(&path, 1), None);
    }

    #[test]
    fn test_poll_file_gone_treated_as_denied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ipc_gone");
        fs::write(&path, b"pending").unwrap();

        let p = path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            fs::remove_file(&p).unwrap();
        });

        assert_eq!(poll_response(&path, 5), Some(false));
    }
}
