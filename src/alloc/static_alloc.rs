use core::mem::{self, MaybeUninit};

pub struct StaticAlloc {
    len: usize,
    pos: usize,
    start: *mut u8,
}

impl StaticAlloc {
    pub(crate) fn new(memory: &'static mut [u8]) -> Self {
        Self {
            len: memory.len(),
            pos: 0,
            start: memory.as_mut_ptr(),
        }
    }

    fn alloc<T>(&mut self) -> Option<&'static mut MaybeUninit<T>> {
        let size = mem::size_of::<T>();
        let align = mem::align_of::<T>();
        let new_pos = round_up(self.pos, align);
        if new_pos + size < self.len {
            self.pos = new_pos + size;
            Some(unsafe { &mut *(self.start.add(new_pos) as *mut MaybeUninit<T>) })
        } else {
            // OOM
            None
        }
    }

    /// Effectively stores `val` in static memory and returns a reference to it
    pub(crate) fn alloc_init<T>(&mut self, val: T) -> Option<&'static mut T> {
        if let Some(slot) = self.alloc::<T>() {
            unsafe {
                slot.as_mut_ptr().write(val);
                Some(&mut *slot.as_mut_ptr())
            }
        } else {
            None
        }
    }
}

/// Rounds up `n` a the nearest multiple `m`
fn round_up(n: usize, m: usize) -> usize {
    let rem = n % m;
    if rem == 0 {
        n
    } else {
        (n + m) - rem
    }
}