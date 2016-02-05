#![cfg_attr(test, deny(warnings))]
#![deny(missing_docs)]

//! # shared-mutex
//!
//! A RwLock that can be used with a Condvar.

use std::sync::{Mutex, Condvar, MutexGuard};
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};

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

/// A raw lock providing both shared read locks and exclusive write locks.
///
/// Used as a raw building block for other synchronization primitives. Most
/// users should just use `SharedMutex<T>`, which takes care of tieing the lock
/// to some data.
pub struct RawSharedMutex {
    state: Mutex<State>,
    readers: Condvar,
    both: Condvar,
}

impl RawSharedMutex {
    /// Create a new RawSharedMutex
    #[inline]
    pub fn new() -> RawSharedMutex {
        RawSharedMutex {
            state: Mutex::new(State::new()),
            readers: Condvar::new(),
            both: Condvar::new()
        }
    }

    /// Acquire a shared read lock.
    ///
    /// Blocks until a read lock can be acquired. The lock can be released
    /// by calling `unlock_read`.
    #[inline]
    pub fn read(&self) {
        self.read_from(self.state.lock().unwrap())
    }

    /// Get a read lock using the given state lock.
    ///
    /// WARNING: The lock MUST be from self.state!!
    fn read_from(&self, mut state_lock: MutexGuard<State>) {
        // Wait for any writers to finish and for there to be space
        // for another reader. (There are a max of 2^63 readers at any time)
        while state_lock.is_writer_active() || state_lock.has_max_readers() {
            state_lock = self.both.wait(state_lock).unwrap();
        }

        // At this point there should be no writers and space
        // for at least one more reader.
        //
        // Add ourselves as a reader.
        state_lock.add_reader();
    }

    /// Acquire an exclusive write lock.
    ///
    /// Blocks until the write lock can be acquired. The lock can be released
    /// by calling `unlock_write`.
    #[inline]
    pub fn write(&self) {
        self.write_from(self.state.lock().unwrap())
    }

    /// Get a write lock using the given state lock.
    ///
    /// WARNING: The lock MUST be from self.state!!
    fn write_from(&self, mut state_lock: MutexGuard<State>) {
        // First wait for any other writers to unlock.
        while state_lock.is_writer_active() {
            state_lock = self.both.wait(state_lock).unwrap();
        }

        // At this point there must be no writers, but there may be readers.
        //
        // We set the writer-active flag so that readers which try to
        // acquire the lock from here on out will be queued after us, to
        // prevent starvation.
        state_lock.set_writer_active();

        // Now wait for all readers to exit.
        //
        // This will happen eventually since new readers are waiting on
        // us because we set the writer-active flag.
        while state_lock.readers() != 0 {
            state_lock = self.readers.wait(state_lock).unwrap();
        }

        // At this point there should be one writer (us) and no readers.
        debug_assert!(state_lock.is_writer_active() && state_lock.readers() == 0,
                      "State not empty on write lock! State = {:?}", *state_lock);
    }

    /// Unlock a previously acquired read lock.
    ///
    /// Behavior is unspecified (but not undefined) if `unlock_read` is called
    /// without a previous accompanying `read`.
    #[inline]
    pub fn unlock_read(&self) {
        let _ = self.unlock_read_to();
    }

    fn unlock_read_to(&self) -> MutexGuard<State> {
        let mut state_lock = self.state.lock().unwrap();

        // First decrement the reader count.
        state_lock.remove_reader();

        // Now check if there is a writer waiting and
        // we are the last reader.
        if state_lock.is_writer_active() {
            if state_lock.readers() == 0 {
                // Wake up the waiting writer.
                self.readers.notify_one();
            }
        // Check if we where at the max number of readers.
        } else if state_lock.near_max_readers() {
            // Wake up a reader to replace us.
            self.both.notify_one()
        }

        // Return the lock for potential further use.
        state_lock
    }

    /// Unlock a previously acquired write lock.
    ///
    /// Behavior is unspecified (but not undefined) if `unlock_write` is called
    /// without a previous accompanying `write`.
    #[inline]
    pub fn unlock_write(&self) {
        let _ = self.unlock_write_to();
    }

    #[inline]
    fn unlock_write_to(&self) -> MutexGuard<State> {
        let mut state_lock = self.state.lock().unwrap();

        // Writer locks are exclusive so we know we can just
        // set the state to empty.
        *state_lock = State::new();

        state_lock
    }

    /// Wait on the given condition variable, resuming with a write lock.
    ///
    /// Behavior is unspecified if there was no previous accompanying `read`.
    #[inline]
    #[inline]
    pub fn wait_from_read_to_write(&self, cond: &Condvar) {
        let state_lock = self.unlock_read_to();
        let state_lock = cond.wait(state_lock).unwrap();
        self.write_from(state_lock);
    }

    /// Wait on the given condition variable, resuming with another read lock.
    ///
    /// Behavior is unspecified if there was no previous accompanying `read`.
    #[inline]
    pub fn wait_from_read_to_read(&self, cond: &Condvar) {
        let state_lock = self.unlock_read_to();
        let state_lock = cond.wait(state_lock).unwrap();
        self.read_from(state_lock);
    }

    /// Wait on the given condition variable, resuming with a read lock.
    ///
    /// Behavior is unspecified if there was no previous accompanying `write`.
    #[inline]
    pub fn wait_from_write_to_read(&self, cond: &Condvar) {
        let state_lock = self.unlock_write_to();
        let state_lock = cond.wait(state_lock).unwrap();
        self.read_from(state_lock);
    }

    /// Wait on the given condition variable, resuming with another write lock.
    ///
    /// Behavior is unspecified if there was no previous accompanying `write`.
    #[inline]
    pub fn wait_from_write_to_write(&self, cond: &Condvar) {
        let state_lock = self.unlock_write_to();
        let state_lock = cond.wait(state_lock).unwrap();
        self.write_from(state_lock);
    }
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
    /// Wait on the given condition variable, and resume with a write lock.
    ///
    /// See the documentation for `std::sync::Condvar::wait` for more information.
    pub fn wait_for_write(&self, cond: &Condvar) -> SharedMutexWriteGuard<'mutex, T> {
        self.mutex.raw.wait_from_read_to_write(cond);

        SharedMutexWriteGuard {
            data: unsafe { &mut *self.mutex.data.get() },
            mutex: self.mutex
        }
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
    /// Wait on the given condition variable, and resume with another write lock.
    pub fn wait_for_write(self, cond: &Condvar) -> Self {
        self.mutex.raw.wait_from_write_to_write(cond);

        self
    }

    /// Wait on the given condition variable, and resume with a read lock.
    pub fn wait_for_read(self, cond: &Condvar) -> SharedMutexReadGuard<'mutex, T> {
        self.mutex.raw.wait_from_write_to_read(cond);

        SharedMutexReadGuard {
            data: self.data,
            mutex: self.mutex
        }
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

/// Internal State of the SharedMutex.
///
/// The high bit indicates if a writer is active.
///
/// The lower bits are used to count the number of readers.
#[derive(Debug)]
struct State(usize);

#[cfg(target_pointer_width = "64")]
const USIZE_BITS: u8 = 64;

#[cfg(target_pointer_width = "32")]
const USIZE_BITS: u8 = 32;

// Only the high bit is set.
//
// We can mask the State with this to see if the
// high bit is set, which would indicate a writer
// is active.
const WRITER_ACTIVE: usize = 1 << USIZE_BITS - 1;

// All the low bits are set, the high bit is not set.
//
// We can mask the State with this to see how many
// readers there are.
//
// Also the maximum number of readers.
const READERS_MASK: usize = !WRITER_ACTIVE;

impl State {
    #[inline]
    fn new() -> Self { State(0) }

    #[inline]
    fn is_writer_active(&self) -> bool { self.0 & WRITER_ACTIVE != 0 }

    #[inline]
    fn set_writer_active(&mut self) { self.0 |= WRITER_ACTIVE }

    #[inline]
    fn readers(&self) -> usize { self.0 & READERS_MASK }

    #[inline]
    fn has_max_readers(&self) -> bool { self.readers() == READERS_MASK }

    #[inline]
    fn near_max_readers(&self) -> bool { self.readers() == READERS_MASK - 1 }

    #[inline]
    fn add_reader(&mut self) { self.0 += 1 }

    #[inline]
    fn remove_reader(&mut self) { self.0 -= 1 }
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

