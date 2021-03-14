#![no_main]
#![no_std]
#![feature(alloc_error_handler)]

#![feature(const_mut_refs, slice_as_chunks)]

#[macro_use]
extern crate enum_map;

extern crate cortex_m;

mod event;
mod rtc;
mod clock;
mod input;
mod midi;
mod output;
mod state;

use embedded_hal::digital::v2::OutputPin;
use rtic::app;
use rtic::cyccnt::U32Ext as _;

use stm32f1xx_hal::gpio::{State, Input, PullUp};
use stm32f1xx_hal::i2c::{BlockingI2c, DutyCycle, Mode};
use stm32f1xx_hal::prelude::*;

use ssd1306::{prelude::*, Builder, I2CDIBuilder};
use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
use stm32f1xx_hal::device::USART1;

use usb_device::bus;

use cortex_m::asm::delay;

use rtt_target::{rprintln, rtt_init_print};

use crate::input::{Scan, Encoder};
use midi::usb;
use crate::midi::{Transmit};

use crate::midi::serial::{SerialMidiIn, SerialMidiOut};
use crate::midi::usb::MidiClass;
use core::result::Result;
use stm32f1xx_hal::serial;

use crate::midi::packet::{MidiPacket, CableNumber};
use crate::midi::Receive;

use crate::state::AppChange::{Patch, Config};

use panic_rtt_target as _;
use stm32f1xx_hal::rtc::Rtc;
use stm32f1xx_hal::serial::{Tx, Rx, StopBits};
use stm32f1xx_hal::gpio::gpioa::{PA6, PA7};
use stm32f1xx_hal::timer::{Event, Timer};
use crate::midi::message::{Channel, Velocity};
use core::convert::TryFrom;
use crate::state::AppState;
use crate::output::Display;
use crate::clock::{CPU_FREQ, PCLK1_FREQ};
use crate::event::{UiEvent, ButtonEvent};

const BLINK_PERIOD: u32 = 20_000_000;
const CTL_SCAN: u32 = 7200;


#[app(device = stm32f1xx_hal::pac, peripherals = true, monotonic = rtic::cyccnt::CYCCNT)]
const APP: () = {
    struct Resources {
        // clock: rtc::RtcClock,
        controls: Encoder<PA6<Input<PullUp>>, PA7<Input<PullUp>>>,
        state: state::AppState,
        display: output::Display,
        usb_midi: midi::usb::UsbMidi,
        din_midi_in: SerialMidiIn<Rx<USART1>>,
        din_midi_out: SerialMidiOut<Tx<USART1>>,
    }

    #[init(schedule = [blink, control_scan])]
    fn init(ctx: init::Context) -> init::LateResources {
        // for some RTIC reason statics need to go first
        static mut USB_BUS: Option<bus::UsbBusAllocator<UsbBusType>> = None;

        rtt_init_print!();

        // unsafe { ALLOCATOR.init(cortex_m_rt::heap_start() as usize, HEAP_SIZE) }
        // rprintln!("Allocator OK");

        // Enable cycle counter
        let mut core = ctx.core;
        core.DWT.enable_cycle_counter();

        let peripherals: stm32f1xx_hal::stm32::Peripherals = ctx.device;

        // Setup clocks
        let mut flash = peripherals.FLASH.constrain();
        let mut rcc = peripherals.RCC.constrain();
        let mut afio = peripherals.AFIO.constrain(&mut rcc.apb2);
        let clocks = rcc
            .cfgr
            .use_hse(8.mhz())
            // maximum CPU overclock
            .sysclk(CPU_FREQ.hz())
            .pclk1(PCLK1_FREQ.hz())
            .freeze(&mut flash.acr);

        assert!(clocks.usbclk_valid());

        rprintln!("Clocks OK");

        // Setup RTC
        // let mut pwr = peripherals.PWR;
        // let mut backup_domain = rcc.bkp.constrain(peripherals.BKP, &mut rcc.apb1, &mut pwr);
        // let rtc = Rtc::rtc(peripherals.RTC, &mut backup_domain);
        // let clock = rtc::RtcClock::new(rtc);

        rprintln!("RTC OK");

        // Get GPIO busses
        let mut gpioa = peripherals.GPIOA.split(&mut rcc.apb2);
        let mut gpiob = peripherals.GPIOB.split(&mut rcc.apb2);
        let mut gpioc = peripherals.GPIOC.split(&mut rcc.apb2);

        // // Setup LED
        let mut onboard_led = gpioc
            .pc13
            .into_push_pull_output_with_state(&mut gpioc.crh, State::Low);
        onboard_led.set_low().unwrap();
        ctx.schedule
            .blink(ctx.start + BLINK_PERIOD.cycles())
            .unwrap();

        rprintln!("Blinker OK");

        let mut timer3 = Timer::tim3(peripherals.TIM3, &clocks, &mut rcc.apb1)
            .start_count_down(input::SCAN_FREQ_HZ.hz());
        timer3.listen(Event::Update);

        // Setup Encoders
        let encoder: Encoder<PA6<Input<PullUp>>, PA7<Input<PullUp>>> = input::encoder(
            event::RotaryId::MAIN,
            gpioa.pa6.into_pull_up_input(&mut gpioa.crl),
            gpioa.pa7.into_pull_up_input(&mut gpioa.crl),
        );
        let _enc_push = gpioa.pa5.into_pull_down_input(&mut gpioa.crl);

        ctx.schedule.control_scan(ctx.start + CTL_SCAN.cycles()).unwrap();

        rprintln!("Controls OK");

        // Setup Display
        let scl = gpiob.pb8.into_alternate_open_drain(&mut gpiob.crh);
        let sda = gpiob.pb9.into_alternate_open_drain(&mut gpiob.crh);

        let i2c = BlockingI2c::i2c1(
            peripherals.I2C1,
            (scl, sda),
            &mut afio.mapr,
            Mode::Fast {
                frequency: 400_000.hz(),
                duty_cycle: DutyCycle::Ratio2to1,
            },
            clocks,
            &mut rcc.apb1,
            1000,
            10,
            1000,
            1000,
        );
        let oled_i2c = I2CDIBuilder::new().init(i2c);
        let mut oled: GraphicsMode<_> = Builder::new().connect(oled_i2c).into();
        oled.init().unwrap();

        output::draw_logo(&mut oled);

        rprintln!("Screen OK");

        // Configure serial
        let tx_pin = gpioa.pa9.into_alternate_push_pull(&mut gpioa.crh);
        let rx_pin = gpioa.pa10;


        // Configure Midi
        let (tx, rx) = serial::Serial::usart1(
            peripherals.USART1,
            (tx_pin, rx_pin),
            &mut afio.mapr,
            serial::Config::default()
                .baudrate(31250.bps())
                // .wordlength(WordLength::DataBits8)
                .stopbits(StopBits::STOP1)
                .parity_none(),
            clocks,
            &mut rcc.apb2,
        )
            .split();
        let din_midi_out = SerialMidiOut::new(tx);
        let din_midi_in = SerialMidiIn::new(rx, CableNumber::MIN);

        rprintln!("Serial port OK");

        // force USB reset for dev mode (it's a Blue Pill thing)
        let mut usb_dp = gpioa.pa12.into_push_pull_output(&mut gpioa.crh);
        usb_dp.set_low().unwrap();
        delay(clocks.sysclk().0 / 100);

        let usb = Peripheral {
            usb: peripherals.USB,
            pin_dm: gpioa.pa11,
            pin_dp: usb_dp.into_floating_input(&mut gpioa.crh),
        };

        *USB_BUS = Some(UsbBus::new(usb));
        let midi_class = MidiClass::new(USB_BUS.as_ref().unwrap());
        let usb_dev = usb::configure_usb(USB_BUS.as_ref().unwrap());
        rprintln!("USB OK");

        rprintln!("-> Initialized");

        init::LateResources {
            // clock,
            controls: encoder,
            state: state::AppState::default(),
            display: output::Display {
                onboard_led,
                oled,
            },
            usb_midi: midi::usb::UsbMidi::new(
                usb_dev,
                midi_class,
            ),
            din_midi_in,
            din_midi_out,
        }
    }

    /// RTIC defaults to SLEEP_ON_EXIT on idle, which is very eco-friendly (much wattage - wow)
    /// Except that sleeping FUCKS with RTT logging, debugging, etc.
    /// Override this with a puny idle loop (such waste!)
    // #[allow(clippy::empty_loop)]
    // #[idle(spawn = [send_din_midi])]
    #[idle]
    fn idle(_ctx: idle::Context) -> ! {
        loop {
        }
    }

    /// USB transmit interrupt
    #[task(binds = USB_HP_CAN_TX, resources = [usb_midi], priority = 3)]
    fn usb_hp_can_tx(ctx: usb_hp_can_tx::Context) {
        let _unhandled = ctx.resources.usb_midi.poll();
    }

    /// USB receive interrupt
    #[task(binds = USB_LP_CAN_RX0, spawn = [send_din_midi], resources = [usb_midi], priority = 3)]
    fn usb_lp_can_rx0(ctx: usb_lp_can_rx0::Context) {
        if ctx.resources.usb_midi.poll() {
            while let Some(packet) = ctx.resources.usb_midi.receive().unwrap() {
                rprintln!("copying USB packet to Serial {:?}", packet);
                ctx.spawn.send_din_midi(packet).unwrap();
            }
        }
    }

    /// Serial receive interrupt
    #[task(binds = USART1, spawn = [send_usb_midi], resources = [din_midi_in], priority = 3)]
    fn serial_rx0(ctx: serial_rx0::Context) {
        while let Some(packet) = ctx.resources.din_midi_in.receive().unwrap() {
            rprintln!("copying Serial packet to USB {:?}", packet);
            // ctx.spawn.send_usb_midi(packet).unwrap();
        }
    }

    /// Encoder scan timer interrupt
    #[task(resources = [controls], spawn = [dispatch_ui], schedule = [control_scan], priority = 1)]
    fn control_scan(ctx: control_scan::Context) {
        let mut encoder = ctx.resources.controls;
        if let Some(event) = encoder.scan(clock::long_now()) {
            ctx.spawn.dispatch_ui(event).unwrap();
        }
        ctx.schedule.control_scan(ctx.scheduled + CTL_SCAN.cycles()).unwrap();
    }

    // #[task(spawn = [redraw], resources = [display, state], capacity = 5, priority = 1)]
    #[task()]
    fn dispatch_ui(ctx: dispatch_ui::Context, event: event::UiEvent) {
        match event {
            UiEvent::Button(but, ButtonEvent::Down(time)) => {}
            UiEvent::Button(but, ButtonEvent::Up(time)) => {}

            UiEvent::Button(but, ButtonEvent::Hold(duration)) => {}
            UiEvent::Button(but, ButtonEvent::Release(duration)) => {}

            UiEvent::Rotary(_, _) => {}
        }
    }

    #[task(resources = [state, display], spawn = [send_din_midi], schedule = [blink])]
    fn blink(ctx: blink::Context) {
        let state: &mut AppState = ctx.resources.state;
        let display: &mut Display = ctx.resources.display;

        state.ui.led_on = !state.ui.led_on;

        let velo = Velocity::try_from(0x7F).unwrap();

        if state.ui.led_on {
            display.onboard_led.set_high().unwrap();
            let note_on = midi::message::MidiMessage::NoteOn(state.arp.channel, state.arp.note, velo);
            ctx.spawn.send_din_midi(note_on.into()).unwrap();
            rprintln!("Send NoteOn ch {:?} note {:?}", state.arp.channel, state.arp.note);

        } else {
            display.onboard_led.set_low().unwrap();
            let note_off = midi::message::MidiMessage::NoteOff(state.arp.channel, state.arp.note, velo);
            ctx.spawn.send_din_midi(note_off.into()).unwrap();
            rprintln!("Sent NoteOff ch {:?}  note {:?}", state.arp.channel, state.arp.note);
            state.arp.bump();
        }
        ctx.schedule
            .blink(ctx.scheduled + BLINK_PERIOD.cycles())
            .unwrap();

    }

    // #[task(spawn = [redraw], resources = [state], capacity = 5)]
    // fn ctl_update(ctx: ctl_update::Context, event: input::Event) {
    //     if let Some(change) = ctx.resources.state.ctl_update(event) {
    //         ctx.spawn.redraw(change).unwrap();
    //     }
    // }

    #[task(resources = [usb_midi], priority = 3)]
    fn send_usb_midi(ctx: send_usb_midi::Context, packet: MidiPacket) {
        if let Err(e) = ctx.resources.usb_midi.transmit(packet) {
            rprintln!("Failed to send USB MIDI: {:?}", e)
        }
    }

    /// Sending Serial MIDI is a slow, _blocking_ operation (for now?).
    /// Use lower priority and enable queuing of tasks (capacity > 1).
    #[task(capacity = 16, priority = 2, resources = [din_midi_out])]
    fn send_din_midi(ctx: send_din_midi::Context, packet: MidiPacket) {
        if let Err(e) = ctx.resources.din_midi_out.transmit(packet) {
            rprintln!("Failed to send Serial MIDI: {:?}", e)
        }
    }

    #[task(resources = [display])]
    fn redraw(ctx: redraw::Context, change: state::AppChange) {
        match change {
            Patch(change) => output::redraw_patch(ctx.resources.display, change),
            Config(change) => output::redraw_config(ctx.resources.display, change),
            _ => {}
        }
    }

    extern "C" {
        // Reuse some interrupts for software task scheduling.
        fn DMA1_CHANNEL5();
        fn DMA1_CHANNEL6();
        fn DMA1_CHANNEL7();
    }
};
