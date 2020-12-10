#![no_std]
#![no_main]

mod async_timer;
mod timer;

use cortex_m_rt::{
    entry,
    exception,
};
use stm32l4xx_hal::{
    self as hal,
    prelude::*,
    interrupt,
    rcc::RccExt,
    stm32::Peripherals,
    pac::TIM15 as ASYNC_TIMER,
    //pac::TIM2 as TEST_TIMER,
    pac::TIM7 as FUCK,
};


use log::LevelFilter;
use rtt_target::rtt_init_print;
use rtt_logger::RTTLogger;
use panic_rtt_target as _;

use drogue_async::init_executor;
use drogue_async::executor;
use drogue_async::task::{spawn, defer};
use stm32l4xx_hal::timer::{Timer, Event};
use crate::async_timer::AsyncTimer;
use stm32l4xx_hal::pac::TIM1;
use cortex_m::peripheral::NVIC;
use log::Level::Debug;
use embedded_time::duration::{Milliseconds, Duration, Seconds};
use embedded_time::rate::{Hertz, Rate, Decihertz, Millihertz, Kilohertz};
use embedded_time::fraction::Fraction;
use embedded_time::fixed_point::FixedPoint;
use stm32l4xx_hal::pac::RCC;

static LOGGER: RTTLogger = RTTLogger::new(LevelFilter::Debug);

#[entry]
fn main() -> ! {
    rtt_init_print!();
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    log::info!("Init");

    let mut device = Peripherals::take().unwrap();

    log::info!("initializing");
    let mut flash = device.FLASH.constrain();
    let mut rcc = device.RCC.constrain();
    let mut pwr = device.PWR.constrain(&mut rcc.apb1r1);
    let clocks = rcc
        .cfgr
        .sysclk(80.mhz())
        .pclk1(80.mhz())
        .pclk2(80.mhz())
        .freeze(&mut flash.acr, &mut pwr);

    init_executor!( memory: 1024 );

    /*
    let mut tim15 = hal::timer::Timer::tim15(
        device.TIM15,
        1.hz(),
        clocks,
        &mut rcc.apb2,
    );
     */

    //let mut test_timer = crate::timer::Timer::tim2( device.TIM2, clocks, &mut rcc.apb1r1);
    let mut tim15 = crate::timer::Timer::tim15(device.TIM15, clocks, &mut rcc.apb2);

    unsafe {
        (&(*RCC::ptr()).apb2enr).modify(|_,w| w.tim15en().set_bit());
        (&(*RCC::ptr()).apb2rstr).modify(|_,w| w.tim15rst().set_bit());
        (&(*RCC::ptr()).apb2rstr).modify(|_,w| w.tim15rst().clear_bit());
    }

    AsyncTimer::initialize(tim15);

    let mut gpioa = device.GPIOA.split(&mut rcc.ahb2);
    let mut ld1 = gpioa
        .pa5
        .into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper);

    spawn("ld1", async move {
        loop {
            ld1.set_high().unwrap();
            AsyncTimer::delay(Milliseconds(1000u32)).await;
            ld1.set_low().unwrap();
            AsyncTimer::delay(Milliseconds(1000u32)).await;
        }
    });

    let mut gpiob = device.GPIOB.split(&mut rcc.ahb2);
    let mut ld2 = gpiob
        .pb14
        .into_push_pull_output(&mut gpiob.moder, &mut gpiob.otyper);

    spawn("ld2", async move {
        loop {
            ld2.set_high().unwrap();
            AsyncTimer::delay(Milliseconds(500u32)).await;
            ld2.set_low().unwrap();
            AsyncTimer::delay(Milliseconds(500u32)).await;
        }
    });

    executor::run_forever();
}





