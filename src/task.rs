use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use crate::executor::{JoinHandle, SpawnError, EXECUTOR};

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
    unsafe {
        EXECUTOR.as_ref().unwrap().spawn(name, future)
    }
}