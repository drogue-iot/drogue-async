use core::future::Future;
use heapless::{
    Vec,
    String,
    consts::*,
};
use std::sync::atomic::{AtomicU8, Ordering};
use std::cell::UnsafeCell;
use crate::alloc::Alloc;
use std::task::{Context, Poll, Waker, RawWakerVTable, RawWaker};
use std::pin::Pin;
use std::alloc::alloc;
use std::marker::PhantomData;
use std::intrinsics::transmute;
use crate::platform;
use std::ops::Deref;
use std::future::Ready;

const TASK_PENDING: u8 = 0;
const TASK_READY: u8 = 1;
const TASK_TERMINATED: u8 = 2;

pub type Spawner<F> = (Box<dyn Task>, JoinHandle<F>);

pub struct Box<T: ?Sized> {
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
            transmute(*self.pointer.get())
        }
    }
}

impl Deref for Box<dyn Task + 'static> {
    type Target = dyn Task;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}


pub struct TaskFuture<F>
    where
        F: ?Sized + Future,
{
    name: String<U16>,
    state: AtomicU8,
    completion_waker: UnsafeCell<Option<Waker>>,
    exit: UnsafeCell<Option<F::Output>>,
    future: UnsafeCell<F>,
}

impl<F> TaskFuture<F>
    where
        F: Future,
        F::Output: Copy,
{
    pub fn new(name: &str, future: F) -> Self {
        TaskFuture {
            name: name.into(),
            state: AtomicU8::new(TASK_READY),
            completion_waker: UnsafeCell::new(None),
            exit: UnsafeCell::new(None),
            future: UnsafeCell::new(future),
        }
    }

    pub fn new_static(alloc: &'static mut Alloc, name: &str, future: F) -> Spawner<F> {
        let task = alloc.alloc_init(
            Self::new(name, future)
        ).unwrap();

        let handle = JoinHandle::new(task);

        (Box::new(task), handle)
    }

    fn future_mut(&self) -> &mut F {
        unsafe {
            &mut (*self.future.get())
        }
    }
}

pub trait Task {
    fn is_ready(&self) -> bool;
    fn mark_ready(&self);

    fn is_pending(&self) -> bool;
    fn mark_pending(&self);

    fn is_terminated(&self) -> bool;
    fn mark_terminated(&self);

    fn get_state_handle(&self) -> *const ();

    fn do_poll(&self);
}

pub trait ValueProvider<R> {
    fn join(&self) -> R;
}

impl<F> Task for TaskFuture<F>
    where F: Future,
          F::Output: Copy,
{
    fn is_ready(&self) -> bool {
        let result = self.state.load(Ordering::Acquire);
        println!("{} is ready? {:?} {}", self.name, &self.state as *const _, result);
        result == TASK_READY
    }

    fn mark_ready(&self) {
        self.state.store(TASK_READY, Ordering::Relaxed);
    }

    fn is_pending(&self) -> bool {
        self.state.load(Ordering::Relaxed) == TASK_PENDING
    }

    fn mark_pending(&self) {
        self.state.store(TASK_PENDING, Ordering::Relaxed);
    }

    fn is_terminated(&self) -> bool {
        self.state.load(Ordering::Relaxed) == TASK_TERMINATED
    }

    fn mark_terminated(&self) {
        self.state.store(TASK_TERMINATED, Ordering::Relaxed);
    }

    fn get_state_handle(&self) -> *const () {
        &self.state as *const _ as *const ()
    }


    fn do_poll(&self) {
        println!("do-poll {}", self.name);
        self.mark_pending();
        let raw_waker = RawWaker::new(self.get_state_handle(), &VTABLE);
        let waker = unsafe { Waker::from_raw(raw_waker) };
        let mut context = Context::from_waker(&waker);

        let f = &mut self.future_mut();
        let pin = unsafe { Pin::new_unchecked(&mut **f) };
        let result = pin.poll(&mut context);

        if let Poll::Ready(val) = result {
            println!("terminated {}", self.name);
            self.mark_terminated();
            unsafe {
                self.exit.get().replace(Some(val));
                println!("waking waiter");
                let waker = &*self.completion_waker.get();
                if let Some(ref waker) = waker {
                    waker.wake_by_ref();
                }
            }
        }
    }
}

pub struct JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy,
{
    task: &'static TaskFuture<F>,
}

impl<F> JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy
{
    pub fn new(task: &'static TaskFuture<F>) -> Self {
        Self {
            task,
        }
    }

    pub fn join(&self) -> F::Output {
        loop {
            // TODO: Add locking
            unsafe {
                let v = &*self.task.exit.get();
                if let Some(val) = v {
                    return *val;
                }
            }
        }
    }
}

impl<F> Future for JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("polling handle for {}", self.task.name);
        unsafe {
            let v = &*self.task.exit.get();
            if let Some(val) = v {
                println!("{} is ready", self.task.name);
                Poll::Ready(*val)
            } else {
                println!("{} is pending", self.task.name);
                (self.task.completion_waker.get()).replace(Some(cx.waker().clone()));
                Poll::Pending
            }
        }
    }
}


pub struct Executor {
    alloc: UnsafeCell<Alloc>,
    tasks: UnsafeCell<Vec<Box<dyn Task>, U8>>,
}

impl Executor {
    pub fn new(memory: &'static mut [u8]) -> Self {
        Self {
            alloc: UnsafeCell::new(Alloc::new(memory)),
            tasks: UnsafeCell::new(Vec::new()),
        }
    }

    pub fn spawn<F>(&self, name: &str, future: F) -> JoinHandle<F>
        where F: Future,
              F::Output: Copy,
    {
        let spawner = TaskFuture::new_static(
            unsafe { &mut *self.alloc.get() },
            name,
            future,
        );
        self.schedule(spawner)
    }

    fn schedule<F>(&self, spawner: Spawner<F>) -> JoinHandle<F>
        where F: ?Sized + Future,
              F::Output: Copy
    {
        unsafe {
            (*self.tasks.get()).push(spawner.0);
        }

        spawner.1
    }

    fn run(&'static self) {
        println!("run!");
        let mut num = 0;
        loop {
            if platform::critical_section(|| {
                // SAFETY: We're inside a global critical section
                unsafe {
                    (*self.tasks.get()).len() == (*self.tasks.get()).iter().filter(|e| e.is_terminated()).count()
                }
            }) {
                println!("terminating");
                return;
            }

            // decl this type just to help IntelliJ
            let ready: Vec<Box<(dyn Task + 'static)>, U8> =
                platform::critical_section(|| {
                    // SAFETY: We're inside a global critical section
                    unsafe { &(*self.tasks.get()) }
                        .iter()
                        .filter(|e| e.is_ready())
                        .map(|e| e.clone())
                        .collect::<Vec<_, U8>>()
                        .clone()
                });


            for t in ready.iter() {
                println!("loop!");
                unsafe {
                    t.do_poll();
                }
            }
            num += 1;
        }
    }
}

// NOTE `*const ()` is &AtomicU8
static VTABLE: RawWakerVTable = {
    unsafe fn clone(p: *const ()) -> RawWaker {
        println!("waker: clone {:?}", p);
        RawWaker::new(p, &VTABLE)
    }
    unsafe fn wake(p: *const ()) {
        println!("waker: wake");
        wake_by_ref(p)
    }
    unsafe fn wake_by_ref(p: *const ()) {
        println!("waker: wake by ref {:?}", p);
        (*(p as *const AtomicU8)).store(TASK_READY, Ordering::Release);
    }
    unsafe fn drop(_: *const ()) {}

    RawWakerVTable::new(clone, wake, wake_by_ref, drop)
};

pub fn spawn<F>(name: &str, future: F) -> JoinHandle<F>
    where F: Future,
          F::Output: Copy, {
    unsafe {
        EXECUTOR.as_ref().unwrap().spawn(name, future)
    }
}

pub fn run() {
    unsafe {
        EXECUTOR.as_ref().unwrap().run()
    }
}

pub static mut EXECUTOR: Option<Executor> = None;

#[macro_export]
macro_rules! init_executor {
    ($size:literal) => {
        static mut EXECUTOR_HEAP: [u8; $size] = [0; $size];

        let executor = Executor::new(
            unsafe {
                &mut EXECUTOR_HEAP
            }
        );

        unsafe {
            $crate::executor::task::EXECUTOR.replace( executor );
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::executor::task::{TaskFuture, Executor, JoinHandle, ValueProvider, spawn, run};
    use crate::alloc::Alloc;

    #[test]
    fn recurse() {
        init_executor!( 1024 );
        let j1 = spawn("root-1", async move {
            let s1 = spawn("sub-1", async move {
                19
            });

            let s2 = spawn("sub-2", async move {
                22
            });

            s1.await;
            s2.await
        });

        let j2 = spawn("root-2", async move {
            "howdy"
        });

        run();

        let v1 = j1.join();
        let v2 = j2.join();

        println!( "v1={}, v2={}", v1, v2);
    }
}