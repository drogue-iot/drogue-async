mod task;

use core::{
    cell::UnsafeCell,
    fmt::{
        Debug,
        Formatter,
    },
    future::Future,
    mem::MaybeUninit,
    sync::atomic::{
        AtomicBool,
        AtomicU8,
        Ordering,
    },
    task::{
        Context,
        Waker,
        RawWaker,
        RawWakerVTable,
        Poll,
    },
    pin::Pin,
};

use heapless::{
    Vec,
    String,
    consts::*,
};

use crate::{
    platform,
    alloc::Alloc,
};
use crate::platform::critical_section;
use std::marker::PhantomData;

pub enum SpawnError {
    Error
}

type Task = Node<dyn Future<Output=()> + 'static>;

pub struct Node<F>
    where
        F: ?Sized,
{
    name: String<U16>,
    ready: AtomicU8,
    future: UnsafeCell<F>,
}

const TASK_PENDING: u8 = 0;
const TASK_READY: u8 = 1;
const TASK_TERMINATED: u8 = 2;

impl Task {
    fn new(alloc: &'static mut Alloc, name: &str, future: impl Future + 'static) -> &'static mut Self {
        alloc.alloc_init(Node {
            name: name.into(),
            ready: AtomicU8::new(TASK_READY),
            future: UnsafeCell::new(async {
                future.await;
                // `spawn`-ed tasks must never terminate
                //crate::abort()
            }),
        }).unwrap()
    }
}

pub trait Executor {
    fn spawn<T>(&'static self, name: &str, future: impl Future<Output=T> + 'static) -> Result<(), SpawnError>;
    fn run(&'static self);
}

pub struct StaticExecutor {
    alloc: UnsafeCell<Alloc>,
    tasks: UnsafeCell<Vec<&'static Task, U8>>,
}

impl StaticExecutor {
    pub fn new(memory: &'static mut [u8]) -> Self {
        Self {
            alloc: UnsafeCell::new(Alloc::new(memory)),
            tasks: UnsafeCell::new(Vec::new()),
        }
    }
}

impl PartialEq for StaticExecutor {
    fn eq(&self, other: &Self) -> bool {
        self as *const StaticExecutor == other as *const StaticExecutor
    }
}

impl Debug for StaticExecutor {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "StaticExecutor@{:?}", self as *const StaticExecutor)
    }
}

impl Executor for StaticExecutor {
    fn spawn<T>(&'static self, name: &str, future: impl Future<Output=T> + 'static) -> Result<(), SpawnError>
    {
        platform::critical_section(|| {
            let task = Task::new(
                unsafe { &mut *self.alloc.get() },
                name,
                future,
            );

            unsafe {
                (*self.tasks.get()).push(
                    task
                ).map_err(|_| SpawnError::Error)
            }
        })
    }

    fn run(&'static self) {
        let mut num = 0;
        loop {
            if platform::critical_section(|| {
                // SAFETY: We're inside a global critical section
                unsafe {
                    (*self.tasks.get()).len() == (*self.tasks.get()).iter().filter(|e| e.ready.load(Ordering::Relaxed) == TASK_TERMINATED).count()
                }
            }) {
                return;
            }

            // decl this type just to help IntelliJ
            let ready: Vec<&Task, U8> =
                platform::critical_section(|| {
                    // SAFETY: We're inside a global critical section
                    unsafe { &(*self.tasks.get()) }
                        .iter()
                        .filter(|e| e.ready.load(Ordering::Relaxed) == TASK_READY)
                        .map(|e| *e)
                        .collect::<Vec<&Task, U8>>()
                        .clone()
                });

            for t in ready.iter() {
                let raw_waker = RawWaker::new(&t.ready as *const _ as *const _, &VTABLE);
                let waker = unsafe { Waker::from_raw(raw_waker) };
                let mut context = Context::from_waker(&waker);
                unsafe {
                    t.ready.store(TASK_PENDING, Ordering::Relaxed);
                    let f = &mut t.future.get();
                    let pin = Pin::new_unchecked(&mut **f);
                    let result = pin.poll(&mut context);
                    if let Poll::Ready(v) = result {
                        t.ready.store(TASK_TERMINATED, Ordering::Relaxed);
                    }
                }
            }
            num += 1;
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

pub(crate) fn current() -> &'static StaticExecutor {
    static INIT: AtomicBool = AtomicBool::new(false);
    static mut EXECUTOR: UnsafeCell<MaybeUninit<StaticExecutor>> = UnsafeCell::new(MaybeUninit::uninit());

    if !platform::in_thread_mode() {
        // tried to access the executor from an interrupt-handler
        platform::abort();
    }

    if INIT.load(Ordering::Relaxed) {
        unsafe { &mut *(EXECUTOR.get() as *mut StaticExecutor) }
    } else {
        unsafe {
            /// Reserved memory for the bump allocator (TODO this could be user configurable)
            static mut MEMORY: [u8; 1024] = [0; 1024];

            let executorp = EXECUTOR.get() as *mut StaticExecutor;
            executorp.write(StaticExecutor::new(&mut MEMORY));
            INIT.store(true, Ordering::Relaxed);
            &mut *executorp
        }
    }
}

pub struct JoinHandle<T> {
    _marker: PhantomData<T>,
}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }
}

pub fn spawn<T>(name: &str, future: impl Future<Output=T> + 'static) -> Result<(), SpawnError>
{
    current().spawn(name, future)
}

pub async fn defer() {
    struct Yield {
        yielded: bool,
    }

    impl Future for Yield {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.yielded {
                Poll::Ready(())
            } else {
                self.yielded = true;
                // wake ourselves
                cx.waker().wake_by_ref();
                unsafe { platform::signal_event_ready(); }
                Poll::Pending
            }
        }
    }

    Yield { yielded: false }.await
}

pub fn run() {
    current().run()
}

#[cfg(test)]
mod tests {
    use crate::executor::{current, Executor, spawn, run, defer};
    use std::sync::atomic::{AtomicU8, Ordering};

    #[test]
    fn executor_initialization() {
        let e1 = current();
        let e2 = current();
        assert_eq!(e1, e2);
    }

    #[test]
    fn spawn_and_run() {
        static counter_foo: AtomicU8 = AtomicU8::new(0);
        static counter_bar: AtomicU8 = AtomicU8::new(0);

        let foo = spawn("foo-task", async {
            for i in 0..20 {
                counter_foo.fetch_add(1, Ordering::Relaxed);
                println!("foo {}", i);
                defer().await;
            }
        });

        let bar = spawn("bar-task", async {
            for i in 0..10 {
                counter_bar.fetch_add(1, Ordering::Relaxed);
                println!("bar {}", i);
                defer().await;
            }
            42
        });

        run();

        assert_eq!(20, counter_foo.load(Ordering::Relaxed));
        assert_eq!(10, counter_bar.load(Ordering::Relaxed));

    }
}
