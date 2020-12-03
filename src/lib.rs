#![cfg_attr(not(test), no_std)]

#![cfg(not(no_std))]
#[macro_use]
extern crate std;

mod executor;
mod alloc;
mod platform;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
