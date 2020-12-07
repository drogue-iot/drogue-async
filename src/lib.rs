#![cfg_attr(not(test), no_std)]
//#![no_std]

#![allow(dead_code)]

#[cfg(not(no_std))]
#[macro_use]
extern crate std;

mod executor;
mod alloc;
mod platform;
pub mod task;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
