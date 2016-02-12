//! Provides ergonomic poisoning primitives for building poisonable structures.

use std::sync::{PoisonError, LockResult};
use std::thread;

/// A typed poisoning wrapper.
///
/// Enforces that access to the contained data respects poisoning.
#[derive(Debug)]
pub struct Poison<T: ?Sized> {
    raw: RawPoison,
    data: T
}

/// A poison guard on an associated Poison.
///
/// If the current thread panics before this instance is dropped, the
/// Poision will become poisoned when this instance drops.
#[derive(Debug)]
pub struct PoisonGuard<'poison, T: ?Sized + 'poison> {
    data: &'poison mut T,
    guard: RawPoisonGuard<'poison>
}

impl<T> Poison<T> {
    /// Create a new Poison in the non-poisoned state.
    #[inline]
    pub fn new(val: T) -> Self {
        Poison {
            raw: RawPoison::new(),
            data: val,
        }
    }

    /// Create a new Poison that is already poisoned.
    #[inline]
    pub fn poisoned(val: T) -> Self {
        Poison {
            raw: RawPoison::poisoned(),
            data: val,
        }
    }

    /// Extract the data from the Poison.
    ///
    /// Returns PoisonError if the Poison is poisoned.
    #[inline]
    pub fn into_inner(self) -> LockResult<T> {
        if self.raw.poisoned {
            Err(PoisonError::new(self.data))
        } else {
            Ok(self.data)
        }
    }
}

impl<T: ?Sized> Poison<T> {
    /// Get a poison lock on this poison.
    ///
    /// Returns PoisonError if the Poison is poisoned.
    #[inline]
    pub fn lock(&mut self) -> LockResult<PoisonGuard<T>> {
        let data = &mut self.data;
        map_result(self.raw.lock(), move |lock| PoisonGuard { data: data, guard: lock })
    }

    /// Get an immutable reference to the data in this poison.
    ///
    /// There is no guard for an immutable reference, since the data must either
    /// be immutable or internally poisoned if it has interior mutability.
    #[inline]
    pub fn get(&self) -> LockResult<&T> {
        if self.raw.poisoned {
            Err(PoisonError::new(&self.data))
        } else {
            Ok(&self.data)
        }
    }

    /// Get a mutable reference without a guard.
    ///
    /// Should only be used in combination with PoisonGuard::into_raw.
    pub unsafe fn get_mut(&mut self) -> &mut T { &mut self.data }
}

impl<'poison, T: ?Sized> PoisonGuard<'poison, T> {
    /// Get an immutable reference to the data.
    pub fn get(&self) -> &T { &self.data }

    /// Get a mutable reference to the data.
    pub fn get_mut(&mut self) -> &mut T { &mut self.data }

    /// Get a reference that escapes the guard.
    ///
    /// Should only be used if the data will be externally poisoned.
    pub unsafe fn into_mut(self) -> &'poison mut T { self.data }

    /// Get the raw poison guard.
    pub fn into_raw(self) -> RawPoisonGuard<'poison> { self.guard }
}

/// A raw poisoning primitive, can be used to build automatically poisoning structures.
#[derive(Debug)]
pub struct RawPoison {
    poisoned: bool
}

/// A guard on a RawPoison.
///
/// If the current thread panics before this instance is dropped, the RawPoison
/// will become poisoned when this instance drops.
#[derive(Debug)]
pub struct RawPoisonGuard<'poison> {
    poison: &'poison mut RawPoison,
    panicking: bool
}

impl RawPoison {
    /// Create a new RawPoison in a non-poisoned state.
    #[inline]
    pub fn new() -> RawPoison {
        RawPoison { poisoned: false }
    }

    /// Create a new RawPoison which is already poisoned.
    #[inline]
    pub fn poisoned() -> RawPoison {
        RawPoison { poisoned: true }
    }

    /// Get a poison lock on this RawPoison.
    ///
    /// If the RawPoison is already poisoned, returns PoisonError.
    #[inline]
    pub fn lock(&mut self) -> LockResult<RawPoisonGuard> {
        let poisoned = self.poisoned;

        let guard = RawPoisonGuard {
            poison: self,
            panicking: thread::panicking()
        };

        if poisoned {
            Err(PoisonError::new(guard))
        } else {
            Ok(guard)
        }
    }
}

impl<'poison> Drop for RawPoisonGuard<'poison> {
    #[inline]
    fn drop(&mut self) {
        if !self.panicking && thread::panicking() {
            self.poison.poisoned = true;
        }
    }
}

/// A simple, useful combinator for dealing with LockResult.
///
/// Applies the action to either the Ok or Err variants
/// of the LockResult and returns a new LockResult in the same
/// state with a new value.
pub fn map_result<T, U, F>(result: LockResult<T>, f: F)
                           -> LockResult<U>
                           where F: FnOnce(T) -> U {
    match result {
        Ok(t) => Ok(f(t)),
        Err(e) => Err(PoisonError::new(f(e.into_inner())))
    }
}
