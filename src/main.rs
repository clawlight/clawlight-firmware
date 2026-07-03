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
//! ## Serial OTA
//!
//! The same USB-Serial-JTAG link also carries firmware updates, so the board
//! never has to be reflashed over a cable after the first bootstrap. A line of
//! the form
//!
//!   "OTA:<len>:<crc32>\n"   (len decimal bytes, crc32 lowercase/upper hex)
//!
//! puts the board into update mode: it streams `len` bytes of a new image in
//! 4 KB blocks into the *inactive* OTA slot, acking each block, verifies the
//! whole image against the CRC-32, then flips the active slot and reboots into
//! it. The running image is never touched, so a dropped or corrupt transfer is
//! harmless — nothing is activated unless the CRC matches. This is the only
//! time the firmware writes back to the host (a one-byte ack per block); it is
//! safe because the daemon is actively reading during a transfer. See
//! docs/esp32-serial-ota.md for the wire protocol and the host side.
//!
//! Requires the two-slot OTA partition table (partitions.csv). On a board
//! still flashed with a single-app layout the OTA path is simply unavailable
//! (the trigger is answered with an error) and the LED behavior is unchanged.
//!
//! The status LEDs live on the Seeed XIAO ESP32-C6's pads D0 / D1 / D2 =
//! GPIO0 / 1 / 2.

#![no_std]
#![no_main]

use embedded_storage::{ReadStorage, Storage};
use esp_backtrace as _;
use esp_bootloader_esp_idf::{
    ota::OtaImageState, ota_updater::OtaUpdater, partitions::PARTITION_TABLE_MAX_LEN,
};
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    main,
    system::software_reset,
    usb_serial_jtag::UsbSerialJtag,
    Blocking,
};
use esp_storage::FlashStorage;

esp_bootloader_esp_idf::esp_app_desc!();

const IDLE_TIMEOUT_MS: u32 = 10_000;
const LAMP_TEST_MS: u32 = 2_000;

/// Longest command line we buffer. LED commands are ≤4 bytes; the OTA trigger
/// ("OTA:<len>:<crc32>") is ~30. 64 leaves comfortable room; anything longer
/// is a malformed line and its overflow bytes are dropped.
const LINE_MAX: usize = 64;

/// Image is streamed a flash sector at a time.
const OTA_BLOCK: usize = 4096;
/// Abort a transfer if the host goes quiet mid-stream (ms of no progress).
/// Also bounds how long an ack write waits on a vanished reader.
const OTA_STALL_MS: u32 = 3_000;

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // LED pins on the Seeed XIAO ESP32-C6: GPIO0/1/2 = pads D0/D1/D2, three
    // adjacent pads. All non-strapping and clear of the native USB pins
    // (12/13), UART0 (16/17), and the onboard user LED (GPIO15).
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

    // Flash access for OTA. The partition-table buffer is reused for every OTA
    // operation, so it lives for the whole program rather than per-update.
    let mut flash = FlashStorage::new(peripherals.FLASH);
    let mut pt_buf = [0u8; PARTITION_TABLE_MAX_LEN];

    // The current (not-yet-terminated) command line, applied on '\n'/'\r'.
    let mut line = [0u8; LINE_MAX];
    let mut line_len = 0usize;

    // Set once the daemon's first command arrives — proof the freshly booted
    // image can talk to its controller. Used to confirm the running slot as
    // Valid so a rollback-capable bootloader keeps it (a no-op on the stock
    // espflash bootloader, which has no rollback; harmless either way).
    let mut confirmed = false;

    // Deliberately no writes back to the host during normal operation: a
    // blocking TX write stalls forever if nothing on the USB side is reading,
    // and the daemon only sends. RX is safe — read_byte() never blocks. The
    // one exception is OTA update mode, entered below, where the daemon is
    // actively reading our per-block acks.
    let mut idle_ms: u32 = 0;
    loop {
        match usb_serial.read_byte() {
            Ok(byte) => {
                // Any byte means the daemon is alive — reset the idle timer.
                idle_ms = 0;
                match byte {
                    b'\n' | b'\r' => {
                        let cmd = &line[..line_len];
                        if cmd.starts_with(b"OTA:") {
                            // Update mode owns the port until it returns (or
                            // reboots on success). LEDs are left as-is.
                            handle_update(&cmd[4..], &mut flash, &mut pt_buf, &mut usb_serial, &delay);
                        } else {
                            if !confirmed {
                                confirm_image(&mut flash, &mut pt_buf);
                                confirmed = true;
                            }
                            apply_leds(cmd, &mut red, &mut yellow, &mut green);
                        }
                        line_len = 0;
                    }
                    _ => {
                        // Accumulate; silently drop overflow past LINE_MAX.
                        if line_len < LINE_MAX {
                            line[line_len] = byte;
                            line_len += 1;
                        }
                    }
                }
            }
            // Error type is Infallible in practice, so this is only WouldBlock.
            Err(_) => {
                delay.delay_millis(1);
                idle_ms = idle_ms.saturating_add(1);
                if idle_ms == IDLE_TIMEOUT_MS {
                    red.set_high();
                    yellow.set_high();
                    green.set_high();
                    // Drop any half-received line so it can't apply later.
                    line_len = 0;
                }
            }
        }
    }
}

/// Apply a completed command line to the LEDs: every `R`/`Y`/`G` (`B` = yellow)
/// present turns its LED on, every one absent turns it off. Unknown bytes are
/// ignored, so an empty line or a line of only unknown bytes = all off.
fn apply_leds(line: &[u8], red: &mut Output<'_>, yellow: &mut Output<'_>, green: &mut Output<'_>) {
    let (mut want_red, mut want_yellow, mut want_green) = (false, false, false);
    for &byte in line {
        match byte {
            b'R' => want_red = true,
            b'Y' | b'B' => want_yellow = true,
            b'G' => want_green = true,
            _ => {}
        }
    }
    set(red, want_red);
    set(yellow, want_yellow);
    set(green, want_green);
}

/// Best-effort: mark the currently running slot as a known-good image. Only
/// meaningful on a bootloader built with auto-rollback (the stock espflash one
/// is not), and a silent no-op on a board still using a single-app layout
/// (OtaUpdater::new fails when there aren't two OTA slots + otadata).
fn confirm_image(flash: &mut FlashStorage, pt_buf: &mut [u8; PARTITION_TABLE_MAX_LEN]) {
    if let Ok(mut ota) = OtaUpdater::new(flash, pt_buf)
        && let Ok(state) = ota.current_ota_state()
        && (state == OtaImageState::New || state == OtaImageState::PendingVerify)
    {
        let _ = ota.set_current_ota_state(OtaImageState::Valid);
    }
}

/// Receive a firmware image over the serial link and, if it verifies, activate
/// it and reboot. `args` is the trigger line after "OTA:" — "<len>:<crc32>".
///
/// Protocol (board replies are one byte + '\n'):
///   host  OTA:<len>:<crc32>\n     trigger (already parsed into `args`)
///   board K\n                     ready — inactive slot found and big enough
///   host  <=4096 raw bytes        one block
///   board K\n                     block written; send the next
///   ...                           repeat until `len` bytes sent
///   board D\n  then reboot        whole-image CRC matched → activated
///   board E\n                     abort at any point; running image untouched
///
/// Returns to the caller on any error (the LED loop resumes); on success it
/// reboots and never returns.
fn handle_update(
    args: &[u8],
    flash: &mut FlashStorage,
    pt_buf: &mut [u8; PARTITION_TABLE_MAX_LEN],
    usb: &mut UsbSerialJtag<'_, Blocking>,
    delay: &Delay,
) {
    let Some((len, expected_crc)) = parse_trigger(args) else {
        send(usb, delay, b"E\n");
        return;
    };

    // Needs the OTA layout; fails cleanly on a single-app board.
    let mut ota = match OtaUpdater::new(flash, pt_buf) {
        Ok(ota) => ota,
        Err(_) => {
            send(usb, delay, b"E\n");
            return;
        }
    };

    // Stream into the inactive slot. Scope the FlashRegion borrow so `ota` is
    // free afterward for activate_next_partition().
    let mut crc = 0xFFFF_FFFFu32;
    {
        let (mut slot, _subtype) = match ota.next_partition() {
            Ok(next) => next,
            Err(_) => {
                send(usb, delay, b"E\n");
                return;
            }
        };
        if len as usize > slot.capacity() {
            send(usb, delay, b"E\n");
            return;
        }
        send(usb, delay, b"K\n"); // ready

        let mut block = [0u8; OTA_BLOCK];
        let mut offset = 0u32;
        let mut remaining = len as usize;
        while remaining > 0 {
            let n = core::cmp::min(OTA_BLOCK, remaining);
            if !read_exact(usb, delay, &mut block[..n]) {
                send(usb, delay, b"E\n"); // host stalled; nothing activated
                return;
            }
            if slot.write(offset, &block[..n]).is_err() {
                send(usb, delay, b"E\n");
                return;
            }
            crc = crc32_update(crc, &block[..n]);
            offset += n as u32;
            remaining -= n;
            send(usb, delay, b"K\n");
        }
    }

    if (crc ^ 0xFFFF_FFFF) != expected_crc {
        send(usb, delay, b"E\n"); // corrupt transfer; running image is safe
        return;
    }
    if ota.activate_next_partition().is_err() {
        send(usb, delay, b"E\n");
        return;
    }
    // Mark the new slot as freshly installed. On a rollback-capable bootloader
    // this arms the "must confirm on next boot" check (confirm_image does the
    // confirming); on the stock bootloader it's just bookkeeping.
    let _ = ota.set_current_ota_state(OtaImageState::New);

    send(usb, delay, b"D\n");
    let _ = usb.flush_tx();
    software_reset(); // boots the just-written slot; never returns
}

/// Parse "<len>:<crc32>" into (length in bytes, expected CRC-32).
fn parse_trigger(args: &[u8]) -> Option<(u32, u32)> {
    let colon = args.iter().position(|&b| b == b':')?;
    let len = parse_dec(&args[..colon])?;
    let crc = parse_hex(&args[colon + 1..])?;
    Some((len, crc))
}

fn parse_dec(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v: u32 = 0;
    for &b in s {
        let d = b.checked_sub(b'0')?;
        if d > 9 {
            return None;
        }
        v = v.checked_mul(10)?.checked_add(d as u32)?;
    }
    Some(v)
}

fn parse_hex(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v: u32 = 0;
    for &b in s {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        v = v.checked_mul(16)?.checked_add(d as u32)?;
    }
    Some(v)
}

/// Read exactly `buf.len()` bytes, returning false if the host goes quiet for
/// OTA_STALL_MS (so a dropped transfer can't wedge the board forever).
fn read_exact(usb: &mut UsbSerialJtag<'_, Blocking>, delay: &Delay, buf: &mut [u8]) -> bool {
    let mut got = 0;
    let mut idle_ms: u32 = 0;
    while got < buf.len() {
        match usb.read_byte() {
            Ok(b) => {
                buf[got] = b;
                got += 1;
                idle_ms = 0;
            }
            Err(_) => {
                delay.delay_millis(1);
                idle_ms += 1;
                if idle_ms >= OTA_STALL_MS {
                    return false;
                }
            }
        }
    }
    true
}

/// Send a short reply and wait (bounded) for it to drain to the host. The
/// bound means a host that vanished mid-update can't block us indefinitely.
fn send(usb: &mut UsbSerialJtag<'_, Blocking>, delay: &Delay, bytes: &[u8]) {
    let _ = usb.write(bytes);
    let mut idle_ms: u32 = 0;
    loop {
        match usb.flush_tx_nb() {
            Ok(()) => break,
            Err(_) => {
                delay.delay_millis(1);
                idle_ms += 1;
                if idle_ms >= OTA_STALL_MS {
                    break;
                }
            }
        }
    }
}

/// Standard CRC-32 (IEEE 802.3 / zlib), bitwise. Seed with 0xFFFFFFFF and XOR
/// the result with 0xFFFFFFFF to finalize — matches the host's crc32.
fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

fn set(led: &mut Output<'_>, on: bool) {
    if on {
        led.set_high();
    } else {
        led.set_low();
    }
}
