# ESP32 status LEDs

clawlight mirrors the aggregate Claude Code session state to a Seeed XIAO
ESP32-C6 over USB serial, driving three LEDs:

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

Target board: **Seeed XIAO ESP32-C6**. The LEDs live on pads **D0 / D1 / D2**
(GPIO0 / 1 / 2). The firmware is in [`../src/main.rs`](../src/main.rs).

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

## Board-specific design decisions (XIAO ESP32-C6)

The XIAO has a **single USB-C port**, wired straight to the C6's built-in
*USB-Serial-JTAG* peripheral (GPIO12/13 internally). One cable both flashes the
firmware and carries the status bytes, and it enumerates with Espressif's
vendor ID (`303a:1001`, shows up as `/dev/cu.usbmodem*` on macOS) — which is
exactly what the clawlight daemon looks for when auto-detecting. There is no
UART-bridge port and no wrong-port choice to make.

LED pin choice — **GPIO0 (red), GPIO1 (yellow), GPIO2 (green)** = pads
**D0 / D1 / D2**:

- They're **three adjacent pads** on one edge of the board, so the whole
  circuit fits in one short row of jumpers.
- They avoid every pin with a side job on the C6: GPIO4/5/8/9/15 are
  strapping pins sampled at reset (8/9 pick the boot mode — pulling these the
  wrong way bricks booting until rewired), GPIO12/13 are the native USB data
  lines (reusing them kills the serial link), GPIO16/17 are UART0, and
  GPIO24–30 are the flash inside the module.
- Embedded nuance: on a microcontroller, *which* pin you pick is rarely
  arbitrary — most pins double as boot-configuration inputs, debug
  interfaces, or bus lines. "Plain GPIO with no reset-time meaning" is
  the thing you're shopping for.

Other XIAO facts worth knowing:

- There's an **onboard user LED on GPIO15** and an antenna RF switch on
  GPIO3/GPIO14 — avoid all three if you ever expand the wiring.
- The board has **BOOT** and **RESET** buttons. If a bad flash ever makes
  the board unresponsive over USB, hold BOOT, tap RESET, release BOOT, and it
  re-enters the ROM bootloader for recovery flashing.

## Parts (from a standard starter kit)

- 1 × breadboard
- 3 × LED: red, yellow, green
- 3 × 330 Ω resistor (220 Ω also fine; see current note below)
- 4 × male-male jumper wires
- Seeed XIAO ESP32-C6 + USB-C cable (solder on the included header pins to
  breadboard it)

## Schematic

Each GPIO sources current through a resistor and LED to ground
("active high"):

```
D0/GPIO0 ───[330Ω]───►├─── ─┐         R = red LED
                    red      │
D1/GPIO1 ───[330Ω]───►├─── ──┼─── GND
                    yellow   │
D2/GPIO2 ───[330Ω]───►├─── ─┘
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

On the XIAO, **D0 / D1 / D2** are the first three pads on one long edge
(USB-C end at the top), and a **GND** pad sits on the opposite edge:

```
Seeed XIAO ESP32-C6 (USB-C at top):

   D0 ○─┐                    ┌─○ 5V
   D1 ○─┼─┐                  ├─○ GND ──────┐
   D2 ○─┼─┼─┐                ├─○ 3V3       │
   D3 ○ │ │ │                ├─○ D10       │  breadboard
   D4 ○ │ │ │                ├─○ D9        │
   D5 ○ │ │ │                └─○ D8        │
   D6 ○ │ │ │                                │
        │ │ └──[330Ω]──►├ RED   (cathode →)──┤
        │ └────[330Ω]──►├ YELLOW (cathode →)─┤
        └──────[330Ω]──►├ GREEN  (cathode →)─┤
                                    (–) rail ─┘
```

Step by step:

1. Solder the included header pins and seat the XIAO across the breadboard's
   center channel.
2. Jumper the board's **GND** pad to the breadboard's blue **(–) rail**.
3. For each LED: pad jumper (D0/D1/D2) → resistor → LED **long leg (anode)**;
   LED **short leg (cathode)** → (–) rail.
4. Nothing connects to 5V or 3V3 — the GPIOs themselves power the LEDs.

## Usage

```bash
# 1. Flash the firmware (see README.md / the flashing runbook)
cargo run --release

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
