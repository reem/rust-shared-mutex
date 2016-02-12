# shared-mutex

> A reader-writer lock that can be used with a Condvar.

## [Documentation](https://crates.fyi/crates/shared-mutex/0.2.3)

Reader writer locks can be more efficient than mutual exclusion locks in cases
where many operations require read-only access to the data, as they can
significantly lower contention by allowing multiple readers to proceed at once.

As a result, most operating system's native libraries come with a reader-writer
lock implementation in addition to mutual exclusion locks. However, on unix
systems, this reader-writer lock (which is used in `std::sync::RwLock`) cannot
be associated with a condition variable, which are limited to the native mutex
(which is used in `std::sync::Mutex`). On Windows the native reader-writer lock
supports waiting on a condition variable but this functionality is not yet
exposed in the standard library.

SharedMutex is an implementation of a reader-writer lock without this (and
other) restrictions - shared mutex guards provide methods for waiting on
condition variables. In addition to this extremely useful new API, this crate
also features some useful combinators, such as mapped guards, and removes other
restrictions by making the guard types `Send` and `Sync`.

The library also provides some other useful APIs, like a `RawSharedMutex` and
utilities for building your own internally poisoned interior mutability types
in the `poison` module (these are used in the implementation of `SharedMutex`).

## Safety

The locking strategy has been adapted from the implementation of
`std::shared_mutex` from libc++ in llvm, and has the same fairness and
starvation guarantees (readers cannot starve writers, waiting writers block
readers).

I have carefully reviewed the code for safety in addition to using automated
tests, but as with all concurrent and unsafe code, more eyes and brains
would be better.

## Usage

Use the crates.io repository; add this to your `Cargo.toml` along
with the rest of your dependencies:

```toml
[dependencies]
shared-mutex = "0.2"
```

## Author

[Jonathan Reem](https://medium.com/@jreem) is the primary author and maintainer of shared-mutex.

## License

MIT

