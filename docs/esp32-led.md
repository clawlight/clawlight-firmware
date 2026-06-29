# ESP32 status LEDs

clawlight mirrors the aggregate Claude Code session state to an ESP32 over USB
serial, driving three LEDs on a breadboard:

| LED    | State        | Meaning                          |
|--------|--------------|----------------------------------|
| Green  | `active`     | All sessions actively working    |
| Yellow | `inactive`   | At least one session is inactive |
| Red    | `needs_help` | At least one session needs help  |
| (off)  | none         | No live sessions                 |

The colors match the macOS menu bar icon exactly: green / yellow / red. The
host side lives in [clawlight](https://github.com/clawlight/clawlight-cli); the
menu bar daemon drives the board automatically once you enable LEDs (press
`l` in the dashboard), and `clawlight led` runs the same driver in the
foreground for debugging.

Reference board: **MuseLab nanoESP32-C6** (ESP32-C6-WROOM-1, dual USB-C).
The firmware in this repo ([`../src/main.rs`](../src/main.rs)) also builds for
the Espressif ESP32-C6-DevKitC-1 and the Seeed XIAO ESP32-C6 — see
[Other boards](#other-boards).

## Boot behavior

On power-up all three LEDs light together for **2 seconds** as a lamp test —
a dead or miswired LED is obvious at a glance — and then the board switches to
the host-reported state. Until the daemon's first command arrives the board
holds all three on; "all on" always means "no host daemon talking," and it's
also where the board lands if the heartbeat (every 2 s) goes silent for 10 s,
so a dead daemon shows as obviously disconnected instead of a stale status.

## Serial protocol

Newline-terminated commands, 115200 baud. Each line names the LEDs that should
be **on**; every LED not named is turned off, so a single line fully describes
the board:

| Codes in line | LEDs lit        |
|---------------|-----------------|
| `R`           | red             |
| `Y`           | yellow          |
| `G`           | green           |
| `RG`          | red + green     |
| `RYG`         | all three       |
| `N` / (empty) | all off         |

The firmware accumulates the letters `R`, `Y`, and `G` (plus `B` as a legacy
alias for yellow — the yellow LED started life as a blue one) until it sees a
`\n`, then applies that set and resets for the next line. Any other byte,
including `N` and stray whitespace, is ignored while accumulating, so a line
with no LED codes resolves to "all off."

This means up to **three** light codes can be combined per command, letting the
host drive any of the eight on/off permutations of the three LEDs. It is also
backward compatible with the original one-byte-per-line protocol: `R\n`,
`Y\n`, `G\n`, and `N\n` each still mean exactly what they used to.

The host resends the current state every 2 seconds as a heartbeat, so the
board converges to the right state after a replug without any handshake. The
firmware ignores unknown bytes, which keeps the protocol forward-compatible.

## Board-specific design decisions (nanoESP32-C6)

The nanoESP32-C6 has **two USB-C ports**, and the choice matters:

- **`ESP32C6` port (next to the RST button)** — wired straight to the
  C6's built-in *USB-Serial-JTAG* peripheral (GPIO12/13 internally). This
  is the port to use: one cable both flashes the firmware and carries the
  status bytes, and it enumerates with Espressif's vendor ID
  (`303a:1001`, shows up as `/dev/cu.usbmodem*` on macOS), which is the
  first thing the clawlight daemon looks for when auto-detecting.
- **`CH343` port (next to the BOOT button)** — goes through a CH343P
  USB-UART bridge into UART0 (GPIO16/17). It also works (clawlight
  recognizes the WCH vendor ID `1a86` as a fallback), but it's a second
  code path in the firmware and the bridge's auto-reset circuit can reset
  the chip when the port is opened — or even during USB enumeration. We
  don't use it.

GPIO pin choice — **GPIO18 (red), GPIO19 (yellow), GPIO20 (green)**:

- They sit **side by side on the bottom header**, two pins from a GND, so
  the whole circuit fits in one short row of jumpers:
  `5V · GND · 9 · 18 · 19 · 20 · …` (bottom row, USB end on the left).
- They avoid every pin with a side job on the C6: GPIO4/5/8/9/15 are
  strapping pins sampled at reset (8/9 pick the boot mode — pulling these
  the wrong way bricks booting until rewired), GPIO12/13 are the native
  USB data lines (reusing them kills the serial link), GPIO16/17 are
  UART0 to the CH343, and GPIO24–30 are the flash inside the module.
- Embedded nuance: on a microcontroller, *which* pin you pick is rarely
  arbitrary — most pins double as boot-configuration inputs, debug
  interfaces, or bus lines. "Plain GPIO with no reset-time meaning" is
  the thing you're shopping for.

Other board facts worth knowing:

- There's an onboard **WS2812 addressable RGB LED on GPIO8**. It could
  show all three colors with zero wiring, but it needs a timing-critical
  one-wire protocol driver (RMT peripheral) instead of three `set_high()`
  calls, and GPIO8 is a strapping pin. Discrete LEDs are the better first
  project; the WS2812 is a nice follow-up.
- The BOOT button is GPIO9. If a bad flash ever makes the board
  unresponsive over USB, hold BOOT, tap RST, and it re-enters the ROM
  bootloader for recovery flashing.

## Parts (from a standard starter kit)

- 1 × breadboard
- 3 × LED: red, yellow, green
- 3 × 330 Ω resistor (220 Ω also fine; see current note below)
- 4 × male-male jumper wires
- nanoESP32-C6 + USB-C cable (plugged into the **ESP32C6** port)

## Schematic

Each GPIO sources current through a resistor and LED to ground
("active high"):

```
GPIO18 ───[330Ω]───►├─── ─┐         R = red LED
                  red      │
GPIO19 ───[330Ω]───►├─── ──┼─── GND
                  yellow   │
GPIO20 ───[330Ω]───►├─── ─┘
                  green

►├  = LED, long leg (anode) toward the resistor,
      short leg / flat side (cathode) toward GND
```

Electrical reasoning (the embedded nuances):

- C6 GPIOs swing 0 → 3.3 V. Red and yellow LEDs drop ~2.0–2.1 V, so a
  330 Ω resistor passes (3.3 − 2.1) / 330 ≈ **3.6 mA** — nicely visible.
  Green LEDs in kits are usually the same low-voltage chemistry. (Blue
  and white LEDs drop ~3.0 V+, which is why a blue LED on 330 Ω at 3.3 V
  is nearly invisible — the reason this project's blue became yellow.)
- The resistor is not optional: an LED is a diode, not a resistor — wired
  bare it would draw whatever current the pin can deliver and cook the
  LED, the pin, or both. The safe budget is ~10 mA per pin (the C6 can
  push more, but there's no reason to).
- Resistor on either leg of the LED works; the circuit is a series loop.

## Breadboard layout

The three GPIOs and GND are nearly adjacent on the **bottom header row**
(the row on the same side as the `CH343` silkscreen; USB ports to the
left):

```
nanoESP32-C6, bottom header (USB end → antenna end):
  5V   GND  GPIO9 GPIO18 GPIO19 GPIO20  GPIO21 ...
        │          │      │      │
        │          │      │      │              breadboard
        │          │      │      │     ┌─────────────────────────────┐
        │          │      │      └─────│ d1 ──[330Ω]── d5  ►├ GREEN  │
        │          │      └────────────│ c10──[330Ω]── c14 ►├ YELLOW │
        │          └───────────────────│ b18──[330Ω]── b22 ►├ RED    │
        │                              │      all cathodes → ( – )   │
        └──────────────────────────────│ ( – ) blue ground rail      │
                                       └─────────────────────────────┘
```

Step by step:

1. Seat the board across the breadboard's center channel (or next to the
   breadboard with jumpers, if you haven't soldered headers).
2. Jumper the board's **GND** (bottom row, 2nd pin from the USB end) to
   the breadboard's blue **(–) rail**.
3. For each LED: GPIO jumper → resistor → LED **long leg (anode)**;
   LED **short leg (cathode)** → (–) rail.
4. Nothing connects to 5V or 3V3 — the GPIOs themselves power the LEDs.

## Other boards

The firmware is the same for any ESP32-C6 — only the LED GPIOs differ,
selected at compile time with a `board-*` cargo feature. Flashing,
toolchain, and the host daemon are identical across boards (the daemon
auto-detects by USB vendor ID, and every C6's native USB enumerates as
Espressif `303a:1001`).

| Board | Feature (default = nano) | LED pins | Wire to pads | USB port |
|-------|--------------------------|----------|--------------|----------|
| MuseLab nanoESP32-C6 | `board-nano` | GPIO18/19/20 | header 18/19/20 | `ESP32C6` (native) |
| Espressif ESP32-C6-DevKitC-1 | `board-devkitc` | GPIO18/19/20 | header 18/19/20 | `USB` (native) |
| Seeed XIAO ESP32-C6 | `board-xiao` | GPIO0/1/2 | **D0 / D1 / D2** | single USB-C (native) |

Build/flash for a non-default board by disabling the default feature:

```bash
cargo run --release --no-default-features --features board-xiao
```

**XIAO ESP32-C6 specifics:** single USB-C port wired straight to the
native USB-Serial-JTAG — no port choice, no bridge-chip reset quirk. If
espflash can't auto-enter the bootloader, hold **BOOT**, plug in (or tap
**RESET**), release BOOT. LEDs go on pads **D0/D1/D2** (red/yellow/green),
which are three adjacent pads = GPIO0/1/2; the nearest **GND** pad is on
the opposite rail. Avoid GPIO15 (onboard user LED) and GPIO3/GPIO14
(antenna switch) if you ever expand the wiring.

## Usage

```bash
# 1. Flash the firmware (see README.md / the flashing runbook)
#    Default boards (nano / DevKitC):
cargo run --release
#    Seeed XIAO ESP32-C6:
#    cargo run --release --no-default-features --features board-xiao

# 2. Enable LEDs in clawlight: open the dashboard and press `l`.
#    The menu bar daemon auto-detects the board and reconnects on replug.
#    For debugging you can run the driver in the foreground instead:
clawlight led
#    Or pin it to a specific device:
clawlight led --port /dev/cu.usbmodem101
```

On boot the firmware runs the lamp test — all three LEDs on together for
2 seconds — then holds **all three LEDs on** until the daemon connects and the
LEDs switch to live status. All-on always means "no host daemon talking"; it's
also where the board lands if the heartbeat goes silent for 10 seconds, so a
dead daemon shows as obviously disconnected instead of a stale status.
