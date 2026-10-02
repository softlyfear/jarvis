use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// An OS file lock held until the window exits, including while it is still starting.
/// Keep the file in place: unlinking a locked file would let another copy bypass it.
pub struct InstanceGuard(File);

impl InstanceGuard {
    pub fn acquire(path: &Path) -> io::Result<Option<Self>> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn simultaneous_launches_only_admit_one_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui-instance.lock");
        let start = Arc::new(Barrier::new(8));
        let finish = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8).map(|_| {
            let (path, start, finish) = (path.clone(), start.clone(), finish.clone());
            std::thread::spawn(move || {
                start.wait();
                let guard = InstanceGuard::acquire(&path).unwrap();
                let owns_window = guard.is_some();
                finish.wait(); // keep the winner's lock until every launch has tried
                owns_window
            })
        }).collect();
        assert_eq!(workers.into_iter().map(|worker| worker.join().unwrap()).filter(|owns| *owns).count(), 1);
        // A stale file after an exit never prevents the next launch.
        assert!(path.exists());
        assert!(InstanceGuard::acquire(&path).unwrap().is_some());
    }

    #[test]
    fn lock_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        assert!(InstanceGuard::acquire(&dir.path().join("missing/gui-instance.lock")).is_err());
    }
}
