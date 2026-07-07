//! `Mutex`/`RwLock` accessors that recover from a poisoned lock instead of
//! panicking. The stdlib's `.lock().unwrap()` propagates a *single* panic
//! (while a lock was held) into every subsequent call on that lock forever —
//! for locks touched on the UI render path (every frame) or by a background
//! Tokio task, that turns one bug into a permanently crashed feature.
//!
//! Use `lock_recover`/`read_recover`/`write_recover` instead of
//! `.lock().unwrap()`/`.read().unwrap()`/`.write().unwrap()` everywhere.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(crate) trait MutexExt<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) trait RwLockExt<T> {
    fn read_recover(&self) -> RwLockReadGuard<'_, T>;
    fn write_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutex_recovers_from_poison() {
        let m = Mutex::new(0);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = m.lock_recover();
            panic!("simulated panic while holding the lock");
        }));
        assert_eq!(*m.lock_recover(), 0);
    }

    #[test]
    fn rwlock_recovers_from_poison() {
        let l = RwLock::new(0);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = l.write_recover();
            panic!("simulated panic while holding the write lock");
        }));
        assert_eq!(*l.read_recover(), 0);
    }
}
