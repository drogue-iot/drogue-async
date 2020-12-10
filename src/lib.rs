#![cfg_attr(not(test), no_std)]
//#![no_std]

#![allow(dead_code)]

#[cfg(feature="std")]
#[macro_use]
#[cfg(feature="std")]
extern crate std;

pub extern crate heapless;

pub mod executor;
pub mod task;

mod alloc;
mod platform;

