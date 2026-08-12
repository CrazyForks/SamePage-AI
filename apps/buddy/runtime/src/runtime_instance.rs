#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

use crate::error::{BuddyError, BuddyResult};

const RUNTIME_INSTANCE_LOCK_FILE_NAME: &str = ".runtime.lock";

#[derive(Debug)]
pub(crate) struct BuddyRuntimeInstanceLock {
    #[cfg(unix)]
    _file: File,
}

impl BuddyRuntimeInstanceLock {
    pub(crate) fn acquire(data_dir: &Path) -> BuddyResult<Self> {
        fs::create_dir_all(data_dir)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(data_dir.join(RUNTIME_INSTANCE_LOCK_FILE_NAME))?;

        acquire_runtime_instance_file_lock(file)
    }
}

#[cfg(unix)]
fn acquire_runtime_instance_file_lock(file: File) -> BuddyResult<BuddyRuntimeInstanceLock> {
    // SAFETY: flock only reads the valid file descriptor owned by `file`; the descriptor remains
    // alive in BuddyRuntimeInstanceLock for the entire lock lifetime.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(BuddyRuntimeInstanceLock { _file: file });
    }

    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Err(BuddyError::Runtime(
            "Lexora Buddy runtime is already running".to_owned(),
        ));
    }

    Err(error.into())
}

#[cfg(not(unix))]
fn acquire_runtime_instance_file_lock(_file: File) -> BuddyResult<BuddyRuntimeInstanceLock> {
    Ok(BuddyRuntimeInstanceLock {})
}

#[cfg(test)]
mod tests {
    use super::BuddyRuntimeInstanceLock;

    #[test]
    #[cfg(unix)]
    fn rejects_second_runtime_instance_until_first_lock_is_released() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-runtime-instance-lock-{}",
            uuid::Uuid::new_v4()
        ));
        let first = BuddyRuntimeInstanceLock::acquire(&data_dir).expect("acquire first lock");

        let error = BuddyRuntimeInstanceLock::acquire(&data_dir)
            .expect_err("second runtime lock should be rejected");

        assert!(error.to_string().contains("runtime is already running"));

        drop(first);
        BuddyRuntimeInstanceLock::acquire(&data_dir).expect("reacquire released lock");
        let _ = std::fs::remove_dir_all(data_dir);
    }
}
