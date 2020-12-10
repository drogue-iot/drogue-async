//! Functions for spawning, joining and deferring/yielding task execution.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use crate::executor::{Task, EXECUTOR, TCB};


/// Errors caused during `spawn`.
#[derive(Copy, Clone, Debug)]
pub enum SpawnError {
    /// The executor has not been initialized using `init_executor!(...)`
    ExecutorNotInitialized,

    /// The internal task-list is saturated
    TaskListFull,
}

/// A handle for .await'ing or joining a spawned task.
///
/// If spawned within an async context, .await per usual.
/// If spawned outside of an async context, .join() will
/// spin block waiting for completion.
pub struct JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy,
{
    // TODO: Move storage of exit value within JoinHandle itself and track drops.
    task: &'static TCB<F>,
}

impl<F> JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy
{
    pub(crate) fn new(task: &'static TCB<F>) -> Self {
        Self {
            task,
        }
    }

    /// Block and spin until the spawned task has completed, and retrieve
    /// its termination value.
    pub fn join(&self) -> F::Output {
        while !self.task.is_terminated() {
            log::info!("spin");
            // spin
        }
        // SAFETY: The task is complete and the exit value, if any
        // has already be written and the executor is done with it.
        unsafe {
            (*self.task.exit.get()).unwrap()
        }
    }
}

impl<F> Future for JoinHandle<F>
    where F: Future + ?Sized + 'static,
          F::Output: Copy
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>
    {
        if ! self.task.is_terminated() {
            unsafe {
                (self.task.completion_waker.get()).replace(Some(cx.waker().clone()));
            }
            Poll::Pending
        } else {
            unsafe {
                Poll::Ready( (*self.task.exit.get()).unwrap() )
            }
        }
    }
}

/// Used within an async context to yield or defer to other tasks
/// within the executor. Not named 'yield' because keywordness causes
/// ugliness.
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


/// Spawn a new async task on the executor.
/// The executor must be initialized prior to using this method.
pub fn spawn<F>(name: &str, future: F) -> Result<JoinHandle<F>, SpawnError>
    where F: Future,
          F::Output: Copy, {
    unsafe {
        if EXECUTOR.is_none() {
            Err(SpawnError::ExecutorNotInitialized)
        } else {
            EXECUTOR.as_ref().unwrap().spawn(name, future)
        }
    }
}