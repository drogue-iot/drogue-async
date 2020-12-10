//! Timers

use stm32l4xx_hal::hal::timer::{CountDown, Periodic};
use stm32l4xx_hal::stm32::{TIM15, TIM16, TIM2, TIM6, TIM7};
#[cfg(any(feature = "stm32l4x5", feature = "stm32l4x6", ))]
use crate::stm32::{TIM17, TIM4, TIM5};
use stm32l4xx_hal::pac::RCC;
//use cast::{u16, u32};
//use void::Void;

use stm32l4xx_hal::rcc::{Clocks, APB1R1, APB2};

use embedded_time::{
    fixed_point::FixedPoint,
    duration::{
        Duration,
        Milliseconds,
    },
    rate::{
        Hertz,
        Millihertz,
    },
};

/// Hardware timers
pub struct Timer<TIM> {
    clocks: Clocks,
    expiration: u16,
    tim: TIM,
}

macro_rules! hal {
    ($($TIM:ident: ($tim:ident, $timXen:ident, $timXrst:ident, $apb:ident),)+) => {
        $(
            impl Timer<$TIM> {
                // NOTE(allow) `w.psc().bits()` is safe for TIM{6,7} but not for TIM{2,3,4} due to
                // some SVD omission
                #[allow(unused_unsafe)]
                pub fn start(&mut self, duration: Milliseconds)
                {
                    // pause
                    self.tim.cr1.modify(|_, w| w.cen().clear_bit());

                    let clock_rate: Millihertz<u64> = Hertz(self.clocks.pclk1().0).into();
                    let deadline: Millihertz<u32> = duration.to_rate::<Millihertz>().unwrap();

                    //let frequency = self.timeout.0;
                    //let ticks = self.clocks.pclk1().0 / frequency; // TODO check pclk that timer is on
                    let ticks = *clock_rate.integer() / *deadline.integer() as u64;
                    let psc = ((ticks - 1) / (1 << 16));

                    self.tim.psc.write(|w| unsafe { w.psc().bits(psc as u16) });

                    let arr = ((ticks / (psc + 1)) & 0xFFFF) as u16;

                    self.expiration = arr;

                    self.tim.dier.write(|w| w.uie().clear_bit());
                    self.tim.arr.write(|w| unsafe { w.bits( arr as u32 ) } );

                    // Trigger an update event to load the prescaler value to the clock
                    self.tim.egr.write(|w| w.ug().set_bit());
                    // The above line raises an update event which will indicate
                    // that the timer is already finished. Since this is not the case,
                    // it should be cleared
                    self.clear_update_interrupt_flag();
                    self.tim.dier.write(|w| w.uie().set_bit());

                    // start counter
                    self.tim.cr1.modify(|_, w| w.cen().set_bit().opm().set_bit() );
                }

                //fn wait(&mut self) -> nb::Result<(), Void> {
                    //if self.tim.sr.read().uif().bit_is_clear() {
                        //Err(nb::Error::WouldBlock)
                    //} else {
                        //self.clear_update_interrupt_flag();
                        //Ok(())
                    //}
                //}

                pub fn $tim(tim: $TIM, clocks: Clocks, apb: &mut $apb) -> Self
                {

/*
                    unsafe {
                        (&(*RCC::ptr()).apb2enr).modify(|_,w| w.$timXen().set_bit());
                        (&(*RCC::ptr()).apb2rstr).modify(|_,w| w.$timXrst().set_bit());
                        (&(*RCC::ptr()).apb2rstr).modify(|_,w| w.$timXrst().clear_bit());
                    }
 */
                    // enable and reset peripheral to a clean slate state
                    //apb.enr().modify(|_, w| w.$timXen().set_bit());
                    //apb.rstr().modify(|_, w| w.$timXrst().set_bit());
                    //apb.rstr().modify(|_, w| w.$timXrst().clear_bit());

                    Timer {
                        clocks,
                        tim,
                        expiration: 0,
                    }
                }

                pub fn value(&self) -> u16 {
                    (self.tim.cnt.read().bits() & 0xFFFF) as u16
                }

                fn arr(&self) -> u32 {
                    self.tim.arr.read().bits() & 0xFFFF
                }

                /// Clears Update Interrupt Flag
                pub fn clear_update_interrupt_flag(&mut self) {
                    self.tim.sr.modify(|_, w| w.uif().clear_bit());
                }

                /// Releases the TIM peripheral
                pub fn free(self) -> $TIM {
                    // pause counter
                    self.tim.cr1.modify(|_, w| w.cen().clear_bit());
                    self.tim
                }
            }
        )+
    }
}

hal! {
    TIM2: (tim2, tim2en, tim2rst, APB1R1),
    TIM6: (tim6, tim6en, tim6rst, APB1R1),
    TIM7: (tim7, tim7en, tim7rst, APB1R1),
    TIM15: (tim15, tim15en, tim15rst, APB2),
    TIM16: (tim16, tim16en, tim16rst, APB2),
}

#[cfg(any(feature = "stm32l4x5", feature = "stm32l4x6", ))]
hal! {
    TIM4: (tim4, tim4en, tim4rst, APB1R1),
    TIM5: (tim5, tim5en, tim5rst, APB1R1),
    TIM17: (tim17, tim17en, tim17rst, APB2),
}
