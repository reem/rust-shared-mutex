#![cfg_attr(test, deny(warnings))]
#![deny(missing_docs)]

//! # shared-mutex
//!
//! A RwLock that can be used with a Condvar.

use std::sync::{Mutex, Condvar};
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};

/// A lock providing both shared read locks and exclusive write locks.
///
/// Similar to `std::sync::RwLock`, except that its guards (`SharedMutexReadGuard` and
/// `SharedMutexWriteGuard`) can wait on `std::sync::Condvar`s, which is very
/// useful for implementing efficient concurrent programs.
///
/// Another difference from `std::sync::RwLock` is that the guard types are `Send`.
pub struct SharedMutex<T> {
    state: Mutex<State>,
    readers: Condvar,
    both: Condvar,
    data: UnsafeCell<T>
}

unsafe impl<T: Send> Send for SharedMutex<T> {}
unsafe impl<T: Sync> Sync for SharedMutex<T> {}

/// A shared read guard on a SharedMutex.
pub struct SharedMutexReadGuard<'mutex, T: 'mutex> {
    data: &'mutex T,
    mutex: &'mutex SharedMutex<T>
}

/// An exclusive write guard on a SharedMutex.
pub struct SharedMutexWriteGuard<'mutex, T: 'mutex> {
    data: &'mutex mut T,
    mutex: &'mutex SharedMutex<T>
}

impl<T> SharedMutex<T> {
    /// Create a new SharedMutex protecting the given value.
    pub fn new(value: T) -> Self {
        SharedMutex {
            state: Mutex::new(State::new()),
            readers: Condvar::new(),
            both: Condvar::new(),
            data: UnsafeCell::new(value)
        }
    }

    /// Acquire an exclusive Write lock on the data.
    pub fn write(&self) -> SharedMutexWriteGuard<T> {
        let mut state_lock = self.state.lock().unwrap();

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

        // Create the guard, then release the state lock.
        SharedMutexWriteGuard {
            data: unsafe { &mut *self.data.get() },
            mutex: self
        }
    }

    /// Acquire a shared Read lock on the data.
    pub fn read(&self) -> SharedMutexReadGuard<T> {
        let mut state_lock = self.state.lock().unwrap();

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

        // Create the guard, then release the state lock.
        SharedMutexReadGuard {
            data: unsafe { &*self.data.get() },
            mutex: self
        }
    }

    /// Get a mutable reference to the data without locking.
    ///
    /// Safe since it requires exclusive access to the lock itself.
    pub fn get_mut(&mut self) -> &mut T { unsafe { &mut *self.data.get() } }

    /// Extract the data from the lock and destroy the lock.
    ///
    /// Safe since it requires ownership of the lock.
    pub fn into_inner(self) -> T {
        unsafe { self.data.into_inner() }
    }

    unsafe fn unlock_reader(&self) {
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
    }

    unsafe fn unlock_writer(&self) {
        let mut state_lock = self.state.lock().unwrap();

        // Writer locks are exclusive so we know we can just
        // set the state to empty.
        *state_lock = State::new();
    }
}

impl<'mutex, T> Deref for SharedMutexReadGuard<'mutex, T> {
    type Target = T;

    fn deref(&self) -> &T { self.data }
}

impl<'mutex, T> Deref for SharedMutexWriteGuard<'mutex, T> {
    type Target = T;

    fn deref(&self) -> &T { self.data }
}

impl<'mutex, T> DerefMut for SharedMutexWriteGuard<'mutex, T> {
    fn deref_mut(&mut self) -> &mut T { self.data }
}

impl<'mutex, T> SharedMutexReadGuard<'mutex, T> {
    /// Wait on the given condition variable.
    pub fn wait(self, cond: &Condvar) -> Self {
        // Grab a reference for later.
        let shared = self.mutex;

        // Unlock the reader lock.
        drop(self);

        // Get the mutex, wait on it, then immediately drop the lock we get back.
        let state_lock = shared.state.lock().unwrap();
        drop(cond.wait(state_lock).unwrap());

        // Re-acquire the read lock.
        shared.read()
    }
}

impl<'mutex, T> SharedMutexWriteGuard<'mutex, T> {
    /// Wait on the given condition variable.
    pub fn wait(self, cond: &Condvar) -> Self {
        // Grab a reference for later.
        let shared = self.mutex;

        // Unlock the reader lock.
        drop(self);

        // Get the mutex, wait on it, then immediately drop the lock we get back.
        let state_lock = shared.state.lock().unwrap();
        drop(cond.wait(state_lock).unwrap());

        // Re-acquire the read lock.
        shared.write()
    }
}

impl<'mutex, T> Drop for SharedMutexReadGuard<'mutex, T> {
    fn drop(&mut self) {
        unsafe { self.mutex.unlock_reader() }
    }
}

impl<'mutex, T> Drop for SharedMutexWriteGuard<'mutex, T> {
    fn drop(&mut self) {
        unsafe { self.mutex.unlock_writer() }
    }
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

const WRITER_ACTIVE: usize = 1 << USIZE_BITS - 1;
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

        _is_send_sync::<SharedMutex<()>>();
        _is_send_sync::<SharedMutexReadGuard<()>>();
        _is_send_sync::<SharedMutexWriteGuard<()>>();
    }
}

