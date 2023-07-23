use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> Guard<T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        Guard { lock: self }
    }

    /// Drops the guard, and consequently unlocks the mutex.
    pub fn unlock(guard: Guard<'_, T>) {
        drop(guard);
    }
}

unsafe impl<T> Sync for SpinLock<T> where T: Send {}

pub struct Guard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Guard<'_, T> {
    /// Returns a mutable reference to the underlying data.
    pub fn get(&self) -> &T {
        // SAFETY: If we have a guard, then we have exclusively locked the lock
        unsafe { &*self.lock.value.get() }
    }

    /// Returns a mutable reference to the underlying data.
    pub fn get_mut(&self) -> &mut T {
        // SAFETY: If we have a guard, then we have exclusively locked the lock
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}
