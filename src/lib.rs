//! A statically-allocated nestable async task executor.
//!
//! Currently the executor supports a hard-coded 64 maximum number of tasks.
//! The pseudo-heap is provided through a statically-allocated byte array.
//!
//! Before spawning any tasks, use the `init_executor!(memory: ...)` macro,
//! specifying the number of _bytes_ of memory to be reserved for the executor.
//!
//! Spawn new tasks from either synchronous or asynchronous code using the
//! `spawn(...)` function. This function returns a `JoinHandler` which can be
//! used from non-async code to `join()` the task and wait for its completion
//! and retrieve its return value, or from within an async context, it may be
//! you may use `.await` to wait for it's completion.
//!
//! Within an asyncrhonous context, `defer()` will yield execution back to the
//! executor to allow other tasks an opportunity to run, if desired.
//!
//! To use within an embedded context, include with `default-features = false`
//! and enable the `cortex-m` feature. This is not stupendously ergonomic, and
//! may soon change.

#![cfg_attr(not(test), no_std)]
#![allow(dead_code)]


#[cfg(feature="std")]
#[macro_use]
#[cfg(feature="std")]
extern crate std;

#[doc(hidden)]
pub extern crate heapless;

pub mod executor;
pub mod task;

mod alloc;
mod platform;

