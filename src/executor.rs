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
};
use crate::{
    alloc::Alloc,
    platform,
};

#[derive(Copy, Clone, Debug)]
pub enum SpawnError {
    TaskListFull,
}

const TASK_PENDING: u8 = 0;
const TASK_READY: u8 = 1;
const TASK_TERMINATED: u8 = 2;

type Spawner<F> = (Box<dyn Task>, JoinHandle<F>);


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


struct TCB<F>
    where
        F: ?Sized + Future,
{
    name: String<U16>,
    state: AtomicU8,
    completion_waker: UnsafeCell<Option<Waker>>,
    exit: UnsafeCell<Option<F::Output>>,
    future: UnsafeCell<F>,
}

impl<F> TCB<F>
    where
        F: Future,
        F::Output: Copy,
{
    fn new(name: &str, future: F) -> Self {
        TCB {
            name: name.into(),
            state: AtomicU8::new(TASK_READY),
            completion_waker: UnsafeCell::new(None),
            exit: UnsafeCell::new(None),
            future: UnsafeCell::new(future),
        }
    }

    fn new_static(alloc: &'static mut Alloc, name: &str, future: F) -> Spawner<F> {
        let task = alloc.alloc_init(
            Self::new(name, future)
        ).unwrap();

        let handle = JoinHandle::new(task);

        (Box::new(task), handle)
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn future_mut(&self) -> &mut F {
        &mut (*self.future.get())
    }
}

pub trait Task {
    fn is_ready(&self) -> bool;
    fn signal_ready(&self);

    fn is_pending(&self) -> bool;
    fn signal_pending(&self);

    fn is_terminated(&self) -> bool;
    fn signal_terminated(&self);

    fn get_state_handle(&self) -> *const ();

    fn do_poll(&self);
}

impl<F> Task for TCB<F>
    where F: Future,
          F::Output: Copy,
{
    fn is_ready(&self) -> bool {
        let result = self.state.load(Ordering::Acquire);
        log::trace!("{} is ready? {:?} {}", self.name, &self.state as *const _, result);
        result == TASK_READY
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

    fn get_state_handle(&self) -> *const () {
        &self.state as *const _ as *const ()
    }


    fn do_poll(&self) {
        println!("do-poll {}", self.name);
        self.signal_pending();
        let raw_waker = RawWaker::new(self.get_state_handle(), &VTABLE);
        let waker = unsafe { Waker::from_raw(raw_waker) };
        let mut context = Context::from_waker(&waker);

        let f = unsafe { &mut self.future_mut() };
        let pin = unsafe { Pin::new_unchecked(&mut **f) };
        let result = pin.poll(&mut context);

        if let Poll::Ready(val) = result {
            println!("terminated {}", self.name);
            self.signal_terminated();
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
    task: &'static TCB<F>,
}

impl<F> JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy
{
    fn new(task: &'static TCB<F>) -> Self {
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

    pub fn spawn<F>(&self, name: &str, future: F) -> Result<JoinHandle<F>, SpawnError>
        where F: Future,
              F::Output: Copy,
    {
        let spawner = TCB::new_static(
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
        println!("run!");
        loop {
            let all_terminated = platform::critical_section(|| {
                // SAFETY: We're inside a global critical section
                unsafe {
                    (*self.tasks.get()).len() == (*self.tasks.get()).iter().filter(|e| e.is_terminated()).count()
                }
            });

            if all_terminated {
                log::info!("terminating");
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
                println!("loop!");
                t.do_poll();
            }
        }
    }
}

// NOTE `*const ()` is &AtomicU8
static VTABLE: RawWakerVTable = {
    unsafe fn clone(p: *const ()) -> RawWaker {
        //println!("waker: clone {:?}", p);
        RawWaker::new(p, &VTABLE)
    }
    unsafe fn wake(p: *const ()) {
        //println!("waker: wake");
        wake_by_ref(p)
    }
    unsafe fn wake_by_ref(p: *const ()) {
        //println!("waker: wake by ref {:?}", p);
        (*(p as *const AtomicU8)).store(TASK_READY, Ordering::Release);
    }
    unsafe fn drop(_: *const ()) {}

    RawWakerVTable::new(clone, wake, wake_by_ref, drop)
};

pub async fn defer() {
    struct Defer {
        yielded: bool,
    }

    impl Future for Defer {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.yielded {
                Poll::Ready(())
            } else {
                self.yielded = true;
                // wake ourselves
                cx.waker().wake_by_ref();
                //unsafe { crate::signal_event_ready(); }
                Poll::Pending
            }
        }
    }

    Defer { yielded: false }.await
}

pub fn spawn<F>(name: &str, future: F) -> Result<JoinHandle<F>, SpawnError>
    where F: Future,
          F::Output: Copy, {
    log::error!("spawn!!!");
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
            $crate::executor::EXECUTOR.replace( executor );
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::executor::{Executor, spawn, run, defer};
    use simple_logger::SimpleLogger;


    #[test]
    fn recurse() {
        SimpleLogger::new().init().unwrap();
        log::error!("WHAT");

        init_executor!( 1024 );
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