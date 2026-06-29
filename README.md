# clawlight-firmware

Bare-metal Rust firmware (esp-hal, `no_std`) for the **ESP32-C6** that drives
three status LEDs from [clawlight](https://github.com/clawlight/clawlight-cli).
Builds for the MuseLab nanoESP32-C6 (default), the Espressif
ESP32-C6-DevKitC-1, and the Seeed XIAO ESP32-C6 via `board-*` cargo features.
Wiring, pin choices, per-board notes, and the serial protocol are documented
in [`docs/esp32-led.md`](docs/esp32-led.md); flashing and recovery are in
[`docs/esp32-flashing-runbook.md`](docs/esp32-flashing-runbook.md).

## What it does

On boot all three LEDs light for **2 seconds** as a power-on lamp test, then
the board switches to the host-reported state. It listens on the chip's native
USB-Serial-JTAG port for newline-terminated commands and lights any
combination of the three LEDs accordingly:

| Code | LED         |
|------|-------------|
| `R`  | red on      |
| `Y`  | yellow on   (`B` accepted as a legacy alias) |
| `G`  | green on    |

Each line is a complete picture of which LEDs are on: every code present
turns its LED on, every code absent turns it off. So `R\n` shows red only,
`RG\n` shows red **and** green, `RYG\n` shows all three, and an empty or
unknown line (`\n`, `N\n`) turns everything off. This is backward compatible
with the original one-byte-per-line protocol (`R` / `Y` / `G` / `N`) while
letting the host drive any of the eight permutations of the three LEDs — see
[Serial protocol](docs/esp32-led.md#serial-protocol).

"All three lit" also means **no host daemon**: it's where the board sits after
the lamp test until the first command arrives, and where it returns if the
heartbeat goes silent for 10 s, so a dead daemon shows as obviously
disconnected rather than a stale status.

## One-time setup

The ESP32-C6 is RISC-V, so this builds on plain stable Rust — no espup /
Xtensa toolchain involved. `rust-toolchain.toml` pulls in the
`riscv32imac-unknown-none-elf` target automatically on first build.

You need espflash; 4.x is the version paired with esp-hal 1.x
(3.3.0 is the floor and also works):

```bash
cargo install espflash --locked
```

## Flash

Plug the USB-C cable into the port labeled **ESP32C6** (next to the RST
button) — that's the chip's native USB-Serial-JTAG, used for both
flashing and the status protocol. The other port (CH343) is a plain
UART bridge this firmware doesn't listen on.

```bash
cargo run --release   # builds, flashes, and opens a serial monitor

# Seeed XIAO ESP32-C6 (LEDs on GPIO0/1/2 = pads D0/D1/D2):
cargo run --release --no-default-features --features board-xiao
```

(`cargo run` invokes `espflash flash --monitor` via the runner in
`.cargo/config.toml`. Ctrl-C exits the monitor; the firmware keeps
running.)

On boot you'll see the lamp test: all three LEDs on together for 2 s, then
the board holds all-on until the clawlight daemon connects and the LEDs
switch to live status. See [the flashing runbook](docs/esp32-flashing-runbook.md)
for the full procedure and recovery steps.

## Recovery

If a future bad flash ever makes the native USB port unresponsive:
hold **BOOT**, tap **RST**, release BOOT. The chip re-enters the ROM
download mode and `espflash` can flash it again.

## Embedded notes (for app devs)

- `#![no_std]`/`#![no_main]`: there's no OS — no heap, no `println!`,
  no exit. `main` returns `!` because there is nowhere to return to.
- `esp_hal::init()` hands out a `Peripherals` struct of singletons;
  ownership of `peripherals.GPIO18` moving into `Output::new` is how
  Rust guarantees at compile time that nothing else can drive that pin.
- The watchdog timers (which reboot the chip if firmware wedges) are
  disabled by `esp_hal::init()`'s default config, so a simple poll loop
  is fine without "feeding" anything.
- `esp_app_desc!()` embeds a metadata block the ESP-IDF second-stage
  bootloader looks for; without it, newer espflash refuses the image.
- The main loop polls with a non-blocking `read_byte()` instead of a
  blocking read so the idle-timeout logic can run while no data is
  arriving. The 1 ms sleep keeps the poll from spinning the CPU flat
  out for no reason.
- A command is accumulated byte by byte and only applied on the
  terminating `\n`, so reading the line a byte at a time never produces a
  flicker through partial states.

## License

MIT
