//! Functions for running the executor's loop, once or forever.

use core::{
    cell::UnsafeCell,
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::atomic::{
        AtomicU8,
        Ordering,
    },
    task::{
        Context,
        Poll,
        Waker,
        RawWakerVTable,
        RawWaker,
    },
};
use heapless::{
    Vec,
    String,
    consts::*,
    ArrayLength,
};

use crate::{
    alloc::Alloc,
    platform,
};
use core::sync::atomic::compiler_fence;
use crate::task::{JoinHandle, SpawnError};




const TASK_PENDING: u8 = 0;
const TASK_READY: u8 = 1;
const TASK_TERMINATED: u8 = 2;

type Spawner<F> = (Box<dyn Task>, JoinHandle<F>);


/// Just enough Box to pin tasks.
struct Box<T: ?Sized> {
    pointer: UnsafeCell<*const T>,
}

impl Clone for Box<dyn Task + 'static> {
    fn clone(&self) -> Self {
        Box::new(self.get())
    }
}

impl<T: ?Sized> Box<T> {
    fn new(val: &'static T) -> Self
    {
        Self {
            pointer: UnsafeCell::new(val as *const T)
        }
    }

    fn get(&self) -> &'static T {
        unsafe {
            &*(*self.pointer.get())
        }
    }
}

impl Deref for Box<dyn Task + 'static> {
    type Target = dyn Task;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}


/// Task Control Block
///
/// Tracks the metadata (name), state, waker for it's own completion,
/// it's exit-value once complete, and the future associated with it.
pub(crate) struct TCB<F>
    where
        F: ?Sized + Future,
{
    name: String<U16>,
    state: AtomicU8,
    pub(crate) completion_waker: UnsafeCell<Option<Waker>>,
    pub(crate) exit: UnsafeCell<Option<F::Output>>,
    future: UnsafeCell<F>,
}

impl<F> TCB<F>
    where
        F: Future,
        F::Output: Copy,
{
    /// Create a new task control block.
    fn new(name: &str, future: F) -> Self {
        TCB {
            name: name.into(),
            state: AtomicU8::new(TASK_READY),
            completion_waker: UnsafeCell::new(None),
            exit: UnsafeCell::new(None),
            future: UnsafeCell::new(future),
        }
    }

    /// Allocate within the "heap" a new task control block.
    fn allocate(alloc: &'static mut Alloc, name: &str, future: F) -> Spawner<F> {
        let task = alloc.alloc_init(
            Self::new(name, future)
        ).unwrap();

        let handle = JoinHandle::new(task);

        (Box::new(task), handle)
    }
}

/// Trait to allow easy interaction and boxing of a TCB.
pub(crate) trait Task {
    /// Check the task state flag to determine if ready to run.
    fn is_ready(&self) -> bool;

    /// Signal the task state flag as ready to run.
    fn signal_ready(&self);

    /// Check the task state flag to determine if pending.
    fn is_pending(&self) -> bool;

    /// Signal the task state flag as pending.
    fn signal_pending(&self);

    /// Check the task state flag to determine if terminated.
    fn is_terminated(&self) -> bool;

    /// Signal the task state flag as terminated.
    fn signal_terminated(&self);

    /// Get a handle (pointer) to the state flag.
    fn get_state_flag_handle(&self) -> *const ();

    /// Perform a single poll of the underlying task.
    unsafe fn do_poll(&self);
}

impl<F> Task for TCB<F>
    where F: Future + ?Sized,
          F::Output: Copy,
{
    fn is_ready(&self) -> bool {
        self.state.load(Ordering::Acquire) == TASK_READY
    }

    fn signal_ready(&self) {
        self.state.store(TASK_READY, Ordering::Relaxed);
    }

    fn is_pending(&self) -> bool {
        self.state.load(Ordering::Relaxed) == TASK_PENDING
    }

    fn signal_pending(&self) {
        self.state.store(TASK_PENDING, Ordering::Relaxed);
    }

    fn is_terminated(&self) -> bool {
        self.state.load(Ordering::Relaxed) == TASK_TERMINATED
    }

    fn signal_terminated(&self) {
        self.state.store(TASK_TERMINATED, Ordering::Relaxed);
    }

    fn get_state_flag_handle(&self) -> *const () {
        &self.state as *const _ as *const ()
    }

    unsafe fn do_poll(&self) {
        // About to poll, mark as pending
        self.signal_pending();

        // State flag is an AtomicU8
        let raw_waker = RawWaker::new(self.get_state_flag_handle(), &VTABLE);
        let waker = Waker::from_raw(raw_waker);
        let mut context = Context::from_waker(&waker);

        let f = &mut &mut (*self.future.get());
        let pin = Pin::new_unchecked(&mut **f);
        let result = pin.poll(&mut context);

        if let Poll::Ready(val) = result {
            // If this task is ready then it has completed and terminated
            self.exit.get().replace(Some(val));
            // Ensure the exit value is replaced *before* signaling
            // any waiters that it is available.
            compiler_fence(Ordering::SeqCst);
            self.signal_terminated();
            let waker = &*self.completion_waker.get();
            // If someone is waiting on the JoinHandle, signal it up the line.
            if let Some(ref waker) = waker {
                waker.wake_by_ref();
            }
        }
    }
}


#[doc(hidden)]
pub struct Executor {
    alloc: UnsafeCell<Alloc>,
    tasks: UnsafeCell<Vec<Box<dyn Task>, U64>>,
}

impl Executor {
    pub fn new(memory: &'static mut [u8]) -> Self {
        Self {
            alloc: UnsafeCell::new(Alloc::new(memory)),
            tasks: UnsafeCell::new(Vec::new()),
        }
    }

    pub fn spawn<F>(&self, name: &str, future: F) -> Result<JoinHandle<F>, SpawnError>
        where F: Future,
              F::Output: Copy,
    {
        let spawner = TCB::allocate(
            unsafe { &mut *self.alloc.get() },
            name,
            future,
        );
        self.schedule(spawner)
    }

    fn schedule<F>(&self, spawner: Spawner<F>) -> Result<JoinHandle<F>, SpawnError>
        where F: ?Sized + Future,
              F::Output: Copy
    {
        unsafe {
            (*self.tasks.get()).push(spawner.0).map_err(|_| SpawnError::TaskListFull)?;
        }

        Ok(spawner.1)
    }

    fn run(&'static self) {
        log::trace!("run!");
        loop {
            let all_terminated = platform::critical_section(|| {
                // SAFETY: We're inside a global critical section
                unsafe {
                    (*self.tasks.get()).len() == (*self.tasks.get()).iter().filter(|e| e.is_terminated()).count()
                }
            });

            if all_terminated {
                log::trace!("terminating");
                return;
            }

            // decl this type just to help IntelliJ
            let ready: Vec<Box<(dyn Task + 'static)>, U8> =
                platform::critical_section(|| {
                    // SAFETY: We're inside a global critical section
                    unsafe { &(*self.tasks.get()) }
                        .iter()
                        .filter(|e| e.is_ready())
                        .cloned()
                        .collect::<Vec<_, U8>>()
                });


            for t in ready.iter() {
                // SAFETY: We are the only path calling do_poll, and it only
                // manipulated non-shared or otherwise atomic aspects of itself.
                unsafe {
                    t.do_poll();
                }
            }
        }
    }
}

// NOTE `*const ()` is &AtomicU8
static VTABLE: RawWakerVTable = {
    unsafe fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    unsafe fn wake(p: *const ()) {
        wake_by_ref(p)
    }
    unsafe fn wake_by_ref(p: *const ()) {
        (*(p as *const AtomicU8)).store(TASK_READY, Ordering::Release);
    }
    unsafe fn drop(_: *const ()) {}

    RawWakerVTable::new(clone, wake, wake_by_ref, drop)
};

/// Run one round of the task executor for
/// pending tasks.
pub fn run() {
    // SAFETY: Internally uses a global mutex/critical-section.
    unsafe {
        EXECUTOR.as_ref().unwrap().run()
    }
}

/// Run, forever, rounds of executing pending tasks, or
/// waiting for newly spawned or newly-ready tasks.
///
/// Does not return.
pub fn run_forever() -> ! {
    loop {
        run()
    }
}

#[doc(hidden)]
pub static mut EXECUTOR: Option<Executor> = None;

#[macro_export]
/// Initialize the executor with a specified amount of memory for storing
/// tasks.
macro_rules! init_executor {
    (memory: $mem_bytes:literal) => {
        static mut EXECUTOR_HEAP: [u8; $mem_bytes] = [0; $mem_bytes];
        let executor = $crate::executor::Executor::new(
            unsafe {
                &mut EXECUTOR_HEAP
            },
        );

        unsafe {
            $crate::executor::EXECUTOR.replace( executor );
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::executor::run;
    use simple_logger::SimpleLogger;
    use crate::task::{spawn, defer};


    #[test]
    fn recurse() {
        SimpleLogger::new().init().unwrap();

        init_executor!( memory: 1024 );
        let j1 = spawn("root-1", async move {
            let s1 = spawn("sub-1", async move {
                for i in 1..10_000_000 {
                    if i % 1_000_000 == 0 {
                        defer().await;
                    }
                    continue;
                }
                19
            });

            let s2 = spawn("sub-2", async move {
                for i in 1..10_000_000 {
                    if i % 1_000_000 == 0 {
                        defer().await;
                    }
                    continue;
                }
                22
            });

            s1.unwrap().await;
            s2.unwrap().await
        }).unwrap();

        let j2 = spawn("root-2", async move {
            "howdy"
        }).unwrap();

        run();

        let v1 = j1.join();
        let v2 = j2.join();

        println!("v1={}, v2={}", v1, v2);
    }
}