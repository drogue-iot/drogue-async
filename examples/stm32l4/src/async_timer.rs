use heapless::{
    Vec,
    consts::*,
};
use core::task::{Waker, Context, Poll};
use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::ops::Deref;

use stm32l4xx_hal::pac::TIM15 as TIMER;
use cortex_m::interrupt::CriticalSection;
use stm32l4xx_hal::hal::timer::CountDown;
use stm32l4xx_hal::interrupt;
use stm32l4xx_hal::prelude::*;
use stm32l4xx_hal::time::U32Ext;
use cortex_m::peripheral::NVIC;
use crate::timer::Timer;
use embedded_time::duration::Milliseconds;

pub(crate) static mut ASYNC_TIMER: Option<AsyncTimer> = None;

// ------------------------------------------------------------------------
// ------------------------------------------------------------------------

pub struct AsyncTimer {
    timer: UnsafeCell<Timer<TIMER>>,
    waiters: UnsafeCell<[Option<Waiter>; 8]>,
    next_deadline: UnsafeCell<Option<Milliseconds>>,
}

impl AsyncTimer {
    pub fn initialize(mut timer: Timer<TIMER>) {
        unsafe { NVIC::unmask(stm32l4xx_hal::stm32::Interrupt::TIM15) };

        let timer = Self {
            timer: UnsafeCell::new(timer),
            waiters: UnsafeCell::new([
                None, None, None, None,
                None, None, None, None,
            ]),
            next_deadline: UnsafeCell::new(None),
        };
        unsafe {
            ASYNC_TIMER.replace(timer)
        };
    }

    fn has_expired(&self, index: u8, waker: &Waker) -> bool {
        cortex_m::interrupt::free(|cs| {
            unsafe {
                let waiter = &mut (*self.waiters.get())[index as usize];
                if let Some(waiter) = waiter {
                    if waiter.deadline == Milliseconds(0u32) {
                        (*self.waiters.get())[index as usize] = None;
                        true
                    } else {
                        // not expired, install new waker
                        waiter.waker.replace(waker.clone());
                        false
                    }
                } else {
                    false
                }
            }
        })
    }

    pub async fn delay(deadline: Milliseconds) {
        let f = cortex_m::interrupt::free(|cs| {
            unsafe {
                ASYNC_TIMER.as_ref().unwrap().schedule(deadline)
            }
        });

        f.await
    }

    async fn schedule(&self, deadline: Milliseconds) {
        struct Delay {
            index: u8,
        }

        impl Future for Delay {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                unsafe {
                    if ASYNC_TIMER.as_ref().unwrap().has_expired(self.index, cx.waker()) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }
            }
        }

        let delay = cortex_m::interrupt::free(|cs| {
            unsafe {
                let index = (*self.waiters.get()).iter_mut().enumerate().find(|e| matches!( e, (_, None) )).unwrap().0;
                (*self.waiters.get())[index] = Some(Waiter {
                    deadline,
                    waker: None,
                });

                match (*self.next_deadline.get()) {
                    None => {
                        (*self.next_deadline.get()).replace(deadline);
                        (*self.timer.get()).start(deadline);
                    }
                    Some(next_deadline) => {
                        // TODO: calculate offset based on perhaps unexpired in-progress deadline we just interrupted and how it affects the math
                        if deadline < next_deadline {
                            (*self.next_deadline.get()).replace(deadline);
                            (*self.timer.get()).start(deadline);
                        }
                    }
                }

                Delay {
                    index: index as u8
                }
            }
        });

        delay.await
    }

    pub fn signal(&self, cs: &CriticalSection) {
        unsafe {
            (*self.timer.get()).clear_update_interrupt_flag();
            if let Some(current_deadline) = (*self.next_deadline.get()) {
                let mut next_deadline = None;
                for waiter in (*self.waiters.get()).iter_mut().filter(|e| matches!(e, Some(_))) {
                    if let Some(waiter) = waiter {
                        if waiter.deadline <= current_deadline {
                            waiter.deadline.0 = 0;
                            if let Some(ref waker) = waiter.waker {
                                waker.wake_by_ref();
                            }
                        } else {
                            waiter.deadline = waiter.deadline - current_deadline;
                            match next_deadline {
                                Some(ms) => {
                                    if waiter.deadline < ms {
                                        next_deadline = Some(waiter.deadline)
                                    }
                                }
                                None => {
                                    next_deadline = Some(waiter.deadline);
                                }
                            }
                        }
                    }
                }
                if let Some(next_deadline) = next_deadline {
                    (*self.timer.get()).start(next_deadline);
                } else {
                    (*self.next_deadline.get()).take();
                }
            }
        }
    }
}

struct Waiter {
    waker: Option<Waker>,
    deadline: Milliseconds,
}

impl Future for Waiter {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.deadline == Milliseconds(0u32) {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}