//! A monitor convenience type that couples a SharedMutex and a Condvar.
//!
//! This type is more convenient to use but a little bit less general than
//! SharedMutex since the monitor uses only one condition variable, whereas
//! a SharedMutex can be used with any number of condition variables.

use std::sync::{Condvar, LockResult};
use std::ops::{Deref, DerefMut};
use std::fmt;

use poison;
use {SharedMutex, SharedMutexReadGuard, SharedMutexWriteGuard};

/// A convenience wrapper around a SharedMutex and a Condvar.
///
/// Provides an ergonomic API for locking and waiting on predicates
/// associated with the internal data.
pub struct Monitor<T: ?Sized> {
    cond: Condvar,
    mutex: SharedMutex<T>
}

/// A shared read guard to the data in a Monitor.
pub struct MonitorReadGuard<'mutex, T: ?Sized + 'mutex> {
    guard: SharedMutexReadGuard<'mutex, T>,
    cond: &'mutex Condvar
}

/// An exclusive write guard to the data in a Monitor.
pub struct MonitorWriteGuard<'mutex, T: ?Sized + 'mutex> {
    guard: SharedMutexWriteGuard<'mutex, T>,
    cond: &'mutex Condvar
}

impl<T> Monitor<T> {
    /// Create a new Monitor.
    pub fn new(val: T) -> Monitor<T> {
        Monitor {
            mutex: SharedMutex::new(val),
            cond: Condvar::new()
        }
    }
}

impl<T: ?Sized> Monitor<T> {
    /// Acquire a shared read lock on the monitor.
    pub fn read(&self) -> LockResult<MonitorReadGuard<T>> {
        poison::map_result(self.mutex.read(), |guard| {
            MonitorReadGuard {
                guard: guard,
                cond: &self.cond
            }
        })
    }

    /// Acquire an exclusive write lock on the monitor.
    pub fn write(&self) -> LockResult<MonitorWriteGuard<T>> {
        poison::map_result(self.mutex.write(), |guard| {
            MonitorWriteGuard {
                guard: guard,
                cond: &self.cond
            }
        })
    }

    /// Notify one thread which is waiting on the monitor.
    ///
    /// Note that it is safe but often incorrect to notify without holding any
    /// lock on the monitor, since the predicate may change between a
    /// notification and a predicate check, potentially causing a deadlock.
    #[inline]
    pub fn notify_one(&self) { self.cond.notify_one() }

    /// Notify all threads which are waiting on the monitor.
    ///
    /// Note that it is safe but often incorrect to notify without holding any
    /// lock on the monitor, since the predicate may change between a
    /// notification and a predicate check, potentially causing a deadlock.
    #[inline]
    pub fn notify_all(&self) { self.cond.notify_all() }

    /// Get a reference to the condition variable in this Monitor for external use.
    #[inline]
    pub fn cond(&self) -> &Condvar { &self.cond }
}

impl<'mutex, T: ?Sized> MonitorReadGuard<'mutex, T> {
    /// Wait for a notification on the monitor, then resume with another read guard.
    pub fn wait_for_read(self) -> LockResult<Self> {
        let (guard, cond) = (self.guard, self.cond);
        poison::map_result(guard.wait_for_read(cond), |guard| {
            MonitorReadGuard {
                guard: guard,
                cond: cond
            }
        })
    }

    /// Wait for a notification on the monitor, then resume with a write guard.
    pub fn wait_for_write(self) -> LockResult<MonitorWriteGuard<'mutex, T>> {
        let (guard, cond) = (self.guard, self.cond);
        poison::map_result(guard.wait_for_write(cond), |guard| {
            MonitorWriteGuard {
                guard: guard,
                cond: cond
            }
        })
    }

    /// Notify a thread waiting on the monitor.
    pub fn notify_one(&self) { self.cond.notify_one() }

    /// Notify all threads waiting on the monitor.
    pub fn notify_all(&self) { self.cond.notify_all() }
}

impl<'mutex, T: ?Sized> MonitorWriteGuard<'mutex, T> {
    /// Wait for a notification on the monitor, then resume with another read guard.
    pub fn wait_for_read(self) -> LockResult<MonitorReadGuard<'mutex, T>> {
        let (guard, cond) = (self.guard, self.cond);
        poison::map_result(guard.wait_for_read(cond), |guard| {
            MonitorReadGuard {
                guard: guard,
                cond: cond
            }
        })
    }

    /// Wait for a notification on the monitor, then resume with another write guard.
    pub fn wait_for_write(self) -> LockResult<Self> {
        let (guard, cond) = (self.guard, self.cond);
        poison::map_result(guard.wait_for_write(cond), |guard| {
            MonitorWriteGuard {
                guard: guard,
                cond: cond
            }
        })
    }

    /// Notify a thread waiting on the monitor.
    pub fn notify_one(&self) { self.cond.notify_one() }

    /// Notify all threads waiting on the monitor.
    pub fn notify_all(&self) { self.cond.notify_all() }
}

impl<'mutex, T: ?Sized> Deref for MonitorReadGuard<'mutex, T> {
    type Target = SharedMutexReadGuard<'mutex, T>;

    fn deref(&self) -> &Self::Target { &self.guard }
}

impl<'mutex, T: ?Sized> Deref for MonitorWriteGuard<'mutex, T> {
    type Target = SharedMutexWriteGuard<'mutex, T>;

    fn deref(&self) -> &Self::Target { &self.guard }
}

impl<'mutex, T: ?Sized> DerefMut for MonitorWriteGuard<'mutex, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.guard }
}

impl<'mutex, T: ?Sized> Into<SharedMutexWriteGuard<'mutex, T>> for MonitorWriteGuard<'mutex, T> {
    fn into(self) -> SharedMutexWriteGuard<'mutex, T> { self.guard }
}

impl<'mutex, T: ?Sized> Into<SharedMutexReadGuard<'mutex, T>> for MonitorReadGuard<'mutex, T> {
    fn into(self) -> SharedMutexReadGuard<'mutex, T> { self.guard }
}

impl<T: ?Sized> AsRef<SharedMutex<T>> for Monitor<T> {
    fn as_ref(&self) -> &SharedMutex<T> { &self.mutex }
}

impl<T: ?Sized> AsMut<SharedMutex<T>> for Monitor<T> {
    fn as_mut(&mut self) -> &mut SharedMutex<T> { &mut self.mutex }
}

impl<T> Into<SharedMutex<T>> for Monitor<T> {
    fn into(self) -> SharedMutex<T> { self.mutex }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Monitor<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Monitor")
            .field("mutex", &&self.mutex)
            .finish()
    }
}

impl<'mutex, T: ?Sized + fmt::Debug> fmt::Debug for MonitorReadGuard<'mutex, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MonitorReadGuard")
            .field("data", &self.guard)
            .finish()
    }
}

impl<'mutex, T: ?Sized + fmt::Debug> fmt::Debug for MonitorWriteGuard<'mutex, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MonitorWriteGuard")
            .field("data", &self.guard)
            .finish()
    }
}

