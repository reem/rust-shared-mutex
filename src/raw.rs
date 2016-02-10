use std::sync::{Mutex, Condvar, MutexGuard};

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

    /// Attempt to acquire a shared read lock without blocking.
    ///
    /// Returns true if we succeeded and false if acquiring a read lock would
    /// require blocking.
    pub fn try_read(&self) -> bool {
        let mut state_lock = self.state.lock().unwrap();

        // If there isn't a waiting writer and there is space for another reader
        // we can just take another read lock.
        if !state_lock.is_writer_active() && !state_lock.has_max_readers() {
            state_lock.add_reader();

            // Success!
            true
        } else {
            false // We would have to block
        }
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

    /// Attempt to acquire an exclusive write lock without blocking.
    ///
    /// Returns true if we succeeded and false if acquiring the write lock would
    /// require blocking.
    pub fn try_write(&self) -> bool {
        let mut state_lock = self.state.lock().unwrap();

        // If there are no readers or writers we can just take the lock.
        if !state_lock.is_writer_active() && state_lock.readers() == 0 {
            state_lock.set_writer_active();

            // Success!
            true
        } else {
            false // We would have to block
        }
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

        // Wake any pending readers or writers.
        self.both.notify_all();

        state_lock
    }

    /// Wait on the given condition variable, resuming with a write lock.
    ///
    /// Behavior is unspecified if there was no previous accompanying `read`.
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

