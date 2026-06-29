//! clawlight status-LED firmware for the ESP32-C6.
//!
//! Listens on the chip's native USB-Serial-JTAG port for newline-terminated
//! commands from the clawlight LED daemon. Each command is a *set* of up to
//! three LED codes; every LED named in the line is switched on and every LED
//! not named is switched off, so the host can drive any of the eight
//! permutations of the three LEDs:
//!
//!   'R' = red LED on
//!   'Y' = yellow LED on   ('B' accepted as a legacy alias for yellow)
//!   'G' = green LED on
//!
//! Bytes accumulate until a '\n' (or '\r'), then the accumulated set is
//! applied and cleared for the next line. Any other byte — including 'N',
//! spaces, or stray characters — is ignored while accumulating, so an empty
//! line, or a line of only unknown bytes like "N\n", turns every LED off.
//! This keeps the firmware compatible with the original one-byte-per-line
//! protocol (R / Y / G / N, each on its own line) while also accepting
//! combinations such as "RG\n" (red + green) or "RYG\n" (all three).
//!
//! On boot all three LEDs light for 2 s as a power-on lamp test, then the
//! board switches to the host-reported state on the first command it
//! receives. All three LEDs lit also means "no host daemon": it is the state
//! the board holds after the lamp test until the first command arrives, and
//! the state it returns to if the heartbeat (sent every 2 s) goes missing for
//! IDLE_TIMEOUT_MS — so a dead daemon can't leave a stale status showing.
//!
//! The LED GPIOs are board-selected at compile time (see the `board-*`
//! cargo features):
//!   board-nano / board-devkitc  -> GPIO18 / 19 / 20  (default)
//!   board-xiao                  -> GPIO0 / 1 / 2  (XIAO pads D0 / D1 / D2)

#![no_std]
#![no_main]

// Exactly one board feature must be active.
#[cfg(not(any(
    feature = "board-nano",
    feature = "board-devkitc",
    feature = "board-xiao"
)))]
compile_error!(
    "Select a board: default builds for board-nano; for XIAO use \
     `--no-default-features --features board-xiao`."
);
#[cfg(all(
    feature = "board-xiao",
    any(feature = "board-nano", feature = "board-devkitc")
))]
compile_error!(
    "Enable only one board feature. For XIAO use \
     `--no-default-features --features board-xiao`."
);

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    main,
    usb_serial_jtag::UsbSerialJtag,
};

esp_bootloader_esp_idf::esp_app_desc!();

const IDLE_TIMEOUT_MS: u32 = 10_000;
const LAMP_TEST_MS: u32 = 2_000;

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Board-selected LED pins. All choices are non-strapping and clear of
    // the native USB pins (12/13), UART0 (16/17), and any onboard hardware.
    //   nano/devkitc: GPIO18/19/20 — adjacent on the header, two pins from GND.
    //   xiao:         GPIO0/1/2    — pads D0/D1/D2, three adjacent pads.
    #[cfg(any(feature = "board-nano", feature = "board-devkitc"))]
    let (p_red, p_yellow, p_green) =
        (peripherals.GPIO18, peripherals.GPIO19, peripherals.GPIO20);
    #[cfg(feature = "board-xiao")]
    let (p_red, p_yellow, p_green) =
        (peripherals.GPIO0, peripherals.GPIO1, peripherals.GPIO2);

    let mut red = Output::new(p_red, Level::Low, OutputConfig::default());
    let mut yellow = Output::new(p_yellow, Level::Low, OutputConfig::default());
    let mut green = Output::new(p_green, Level::Low, OutputConfig::default());

    // Power-on lamp test: every LED on, held for 2 s, so a dead or miswired
    // LED is obvious at a glance. We don't read serial during the hold, so
    // the lamp test always lasts the full 2 s; afterwards the loop switches
    // to the host-reported state on the first command (and until then leaves
    // the LEDs on — "all on" means no daemon is talking yet).
    red.set_high();
    yellow.set_high();
    green.set_high();
    delay.delay_millis(LAMP_TEST_MS);

    let mut usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);

    // Which LEDs the current (not-yet-terminated) command line has asked for.
    // Applied and cleared on each '\n'.
    let (mut want_red, mut want_yellow, mut want_green) = (false, false, false);

    // Deliberately no writes back to the host: a blocking TX write stalls
    // forever if nothing on the USB side is reading, and the clawlight daemon
    // only sends. RX is safe — read_byte() never blocks.
    let mut idle_ms: u32 = 0;
    loop {
        match usb_serial.read_byte() {
            Ok(byte) => {
                // Any byte means the daemon is alive — reset the idle timer.
                idle_ms = 0;
                match byte {
                    b'\n' | b'\r' => {
                        set(&mut red, want_red);
                        set(&mut yellow, want_yellow);
                        set(&mut green, want_green);
                        want_red = false;
                        want_yellow = false;
                        want_green = false;
                    }
                    b'R' => want_red = true,
                    b'Y' | b'B' => want_yellow = true,
                    b'G' => want_green = true,
                    // Ignore anything else (e.g. legacy 'N', spaces): a line
                    // with no LED codes resolves to the empty set = all off.
                    _ => {}
                }
            }
            // Error type is Infallible, so this is only ever WouldBlock.
            Err(_) => {
                delay.delay_millis(1);
                idle_ms = idle_ms.saturating_add(1);
                if idle_ms == IDLE_TIMEOUT_MS {
                    red.set_high();
                    yellow.set_high();
                    green.set_high();
                    // Drop any half-received line so it can't apply later.
                    want_red = false;
                    want_yellow = false;
                    want_green = false;
                }
            }
        }
    }
}

fn set(led: &mut Output<'_>, on: bool) {
    if on {
        led.set_high();
    } else {
        led.set_low();
    }
}
