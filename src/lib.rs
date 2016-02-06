#![cfg_attr(test, deny(warnings))]
#![deny(missing_docs)]

//! # shared-mutex
//!
//! A RwLock that can be used with a Condvar.

use std::sync::Condvar;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::{mem, ptr};

pub use raw::RawSharedMutex;

mod raw;

/// A lock providing both shared read locks and exclusive write locks.
///
/// Similar to `std::sync::RwLock`, except that its guards (`SharedMutexReadGuard` and
/// `SharedMutexWriteGuard`) can wait on `std::sync::Condvar`s, which is very
/// useful for implementing efficient concurrent programs.
///
/// Another difference from `std::sync::RwLock` is that the guard types are `Send`.
pub struct SharedMutex<T: ?Sized> {
    raw: RawSharedMutex,
    data: UnsafeCell<T>
}

unsafe impl<T: ?Sized + Send> Send for SharedMutex<T> {}
unsafe impl<T: ?Sized + Sync> Sync for SharedMutex<T> {}

impl<T> SharedMutex<T> {
    /// Create a new SharedMutex protecting the given value.
    #[inline]
    pub fn new(value: T) -> Self {
        SharedMutex {
            raw: RawSharedMutex::new(),
            data: UnsafeCell::new(value)
        }
    }

    /// Extract the data from the lock and destroy the lock.
    ///
    /// Safe since it requires ownership of the lock.
    #[inline]
    pub fn into_inner(self) -> T {
        unsafe { self.data.into_inner() }
    }
}

impl<T: ?Sized> SharedMutex<T> {
    /// Acquire an exclusive Write lock on the data.
    #[inline]
    pub fn write(&self) -> SharedMutexWriteGuard<T> {
        self.raw.write();
        SharedMutexWriteGuard {
            data: unsafe { &mut *self.data.get() },
            mutex: self
        }
    }

    /// Acquire a shared Read lock on the data.
    #[inline]
    pub fn read(&self) -> SharedMutexReadGuard<T> {
        self.raw.read();
        SharedMutexReadGuard {
            data: unsafe { &*self.data.get() },
            mutex: self
        }
    }

    /// Get a mutable reference to the data without locking.
    ///
    /// Safe since it requires exclusive access to the lock itself.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T { unsafe { &mut *self.data.get() } }
}

/// A shared read guard on a SharedMutex.
pub struct SharedMutexReadGuard<'mutex, T: ?Sized + 'mutex> {
    data: &'mutex T,
    mutex: &'mutex SharedMutex<T>
}

unsafe impl<'mutex, T: ?Sized + Send> Send for SharedMutexReadGuard<'mutex, T> {}
unsafe impl<'mutex, T: ?Sized + Sync> Sync for SharedMutexReadGuard<'mutex, T> {}

/// An exclusive write guard on a SharedMutex.
pub struct SharedMutexWriteGuard<'mutex, T: ?Sized + 'mutex> {
    data: &'mutex mut T,
    mutex: &'mutex SharedMutex<T>
}

impl<'mutex, T: ?Sized> Deref for SharedMutexReadGuard<'mutex, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T { self.data }
}

impl<'mutex, T: ?Sized> Deref for SharedMutexWriteGuard<'mutex, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T { self.data }
}

impl<'mutex, T: ?Sized> DerefMut for SharedMutexWriteGuard<'mutex, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T { self.data }
}

impl<'mutex, T: ?Sized> SharedMutexReadGuard<'mutex, T> {
    /// Turn this guard into a guard which can be mapped to a sub-borrow.
    ///
    /// Note that a mapped guard cannot wait on a `Condvar`.
    pub fn into_mapped(self) -> MappedSharedMutexReadGuard<'mutex, T> {
        let guard = MappedSharedMutexReadGuard {
            mutex: &self.mutex.raw,
            data: self.data
        };

        mem::forget(self);

        guard
    }

    /// Wait on the given condition variable, and resume with a write lock.
    ///
    /// See the documentation for `std::sync::Condvar::wait` for more information.
    pub fn wait_for_write(&self, cond: &Condvar) -> SharedMutexWriteGuard<'mutex, T> {
        self.mutex.raw.wait_from_read_to_write(cond);

        let guard = SharedMutexWriteGuard {
            data: unsafe { &mut *self.mutex.data.get() },
            mutex: self.mutex
        };

        // Don't double-unlock.
        mem::forget(self);

        guard
    }

    /// Wait on the given condition variable, and resume with another read lock.
    ///
    /// See the documentation for `std::sync::Condvar::wait` for more information.
    pub fn wait_for_read(self, cond: &Condvar) -> Self {
        self.mutex.raw.wait_from_read_to_read(cond);

        self
    }
}

impl<'mutex, T: ?Sized> SharedMutexWriteGuard<'mutex, T> {
    /// Turn this guard into a guard which can be mapped to a sub-borrow.
    ///
    /// Note that a mapped guard cannot wait on a `Condvar`.
    pub fn into_mapped(self) -> MappedSharedMutexWriteGuard<'mutex, T> {
        let guard = MappedSharedMutexWriteGuard {
            mutex: &self.mutex.raw,
            data: unsafe { &mut *self.mutex.data.get() }
        };

        mem::forget(self);

        guard
    }

    /// Wait on the given condition variable, and resume with another write lock.
    pub fn wait_for_write(self, cond: &Condvar) -> Self {
        self.mutex.raw.wait_from_write_to_write(cond);

        self
    }

    /// Wait on the given condition variable, and resume with a read lock.
    pub fn wait_for_read(self, cond: &Condvar) -> SharedMutexReadGuard<'mutex, T> {
        self.mutex.raw.wait_from_write_to_read(cond);

        let guard = SharedMutexReadGuard {
            data: unsafe { &*self.mutex.data.get() },
            mutex: self.mutex
        };

        // Don't double-unlock.
        mem::forget(self);

        guard
    }
}

impl<'mutex, T: ?Sized> Drop for SharedMutexReadGuard<'mutex, T> {
    #[inline]
    fn drop(&mut self) { self.mutex.raw.unlock_read() }
}

impl<'mutex, T: ?Sized> Drop for SharedMutexWriteGuard<'mutex, T> {
    #[inline]
    fn drop(&mut self) { self.mutex.raw.unlock_write() }
}

/// A read guard to a sub-borrow of an original SharedMutexReadGuard.
///
/// Unlike SharedMutexReadGuard, it cannot be used to wait on a
/// `Condvar`.
pub struct MappedSharedMutexReadGuard<'mutex, T: ?Sized + 'mutex> {
    mutex: &'mutex RawSharedMutex,
    data: &'mutex T
}

/// A write guard to a sub-borrow of an original `SharedMutexWriteGuard`.
///
/// Unlike `SharedMutexWriteGuard`, it cannot be used to wait on a
/// `Condvar`.
pub struct MappedSharedMutexWriteGuard<'mutex, T: ?Sized + 'mutex> {
    mutex: &'mutex RawSharedMutex,
    data: &'mutex mut T
}

impl<'mutex, T: ?Sized> MappedSharedMutexReadGuard<'mutex, T> {
    /// Transform this guard into a sub-borrow of the original data.
    #[inline]
    pub fn map<U, F>(self, action: F) -> MappedSharedMutexReadGuard<'mutex, U>
    where F: FnOnce(&T) -> &U {
        self.option_map(move |t| Some(action(t))).unwrap()
    }

    /// Conditionally transform this guard into a sub-borrow of the original data.
    #[inline]
    pub fn option_map<U, F>(self, action: F) -> Option<MappedSharedMutexReadGuard<'mutex, U>>
    where F: FnOnce(&T) -> Option<&U> {
        self.result_map(move |t| action(t).ok_or(())).ok()
    }

    /// Conditionally transform this guard into a sub-borrow of the original data.
    ///
    /// If the transformation operation is aborted, returns the original guard.
    #[inline]
    pub fn result_map<U, E, F>(self, action: F)
        -> Result<MappedSharedMutexReadGuard<'mutex, U>, (Self, E)>
    where F: FnOnce(&T) -> Result<&U, E> {
        let data = self.data;
        let mutex = self.mutex;

        match action(data) {
            Ok(new_data) => {
                // Don't double-unlock.
                mem::forget(self);

                Ok(MappedSharedMutexReadGuard {
                    data: new_data,
                    mutex: mutex
                })
            },
            Err(e) => { Err((self, e)) }
        }
    }
}

impl<'mutex, T: ?Sized> MappedSharedMutexWriteGuard<'mutex, T> {
    /// Transform this guard into a sub-borrow of the original data.
    #[inline]
    pub fn map<U, F>(self, action: F) -> MappedSharedMutexWriteGuard<'mutex, U>
    where F: FnOnce(&mut T) -> &mut U {
        self.option_map(move |t| Some(action(t))).unwrap()
    }

    /// Conditionally transform this guard into a sub-borrow of the original data.
    #[inline]
    pub fn option_map<U, F>(self, action: F) -> Option<MappedSharedMutexWriteGuard<'mutex, U>>
    where F: FnOnce(&mut T) -> Option<&mut U> {
        self.result_map(move |t| action(t).ok_or(())).ok()
    }

    /// Conditionally transform this guard into a sub-borrow of the original data.
    ///
    /// If the transformation operation is aborted, returns the original guard.
    #[inline]
    pub fn result_map<U, E, F>(self, action: F)
        -> Result<MappedSharedMutexWriteGuard<'mutex, U>, (Self, E)>
    where F: FnOnce(&mut T) -> Result<&mut U, E> {
        let data = unsafe { ptr::read(&self.data) };
        let mutex = self.mutex;

        match action(data) {
            Ok(new_data) => {
                // Don't double-unlock.
                mem::forget(self);

                Ok(MappedSharedMutexWriteGuard {
                    data: new_data,
                    mutex: mutex
                })
            },
            Err(e) => { Err((self, e)) }
        }
    }
}

impl<'mutex, T: ?Sized> Deref for MappedSharedMutexReadGuard<'mutex, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T { self.data }
}

impl<'mutex, T: ?Sized> Deref for MappedSharedMutexWriteGuard<'mutex, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T { self.data }
}

impl<'mutex, T: ?Sized> DerefMut for MappedSharedMutexWriteGuard<'mutex, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T { self.data }
}

impl<'mutex, T: ?Sized> Drop for MappedSharedMutexReadGuard<'mutex, T> {
    #[inline]
    fn drop(&mut self) { self.mutex.unlock_read() }
}

impl<'mutex, T: ?Sized> Drop for MappedSharedMutexWriteGuard<'mutex, T> {
    #[inline]
    fn drop(&mut self) { self.mutex.unlock_write() }
}

#[cfg(test)]
mod test {
    use super::*;

    fn _check_bounds() {
        fn _is_send_sync<T: Send + Sync>() {}

        _is_send_sync::<RawSharedMutex>();
        _is_send_sync::<SharedMutex<()>>();
        _is_send_sync::<SharedMutexReadGuard<()>>();
        _is_send_sync::<SharedMutexWriteGuard<()>>();
    }
}

