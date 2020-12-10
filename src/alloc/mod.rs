
#[cfg(feature="cortex-m")]
mod cortex_m;

#[cfg(feature="cortex-m")]
pub(crate) use self::cortex_m::CortexMHeap as Alloc;

#[cfg(feature = "std")]
mod static_alloc;

#[cfg(feature = "std")]
pub(crate) use self::static_alloc::StaticAlloc as Alloc;