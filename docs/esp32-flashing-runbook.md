# Runbook — flashing the clawlight status-LED firmware

How to build and flash new firmware to the Seeed XIAO ESP32-C6, and how to
recover from the failures we've actually hit. The firmware is in this repo
([`../src/main.rs`](../src/main.rs)); wiring and design are in
[`esp32-led.md`](esp32-led.md).

## TL;DR

```bash
# 1. Free the serial port — whatever is driving the LEDs holds it open and
#    blocks the flasher. In the clawlight dashboard, press `l` to turn LEDs
#    off (the menu bar daemon releases the port within ~2 s).

# 2. Build + flash + monitor (from this repo)
cd ~/Developer/clawlight-firmware
cargo run --release          # Ctrl-C exits the monitor
# Or, from the clawlight-cli repo, one command: scripts/flash.sh

# 3. Re-enable LEDs: press `l` again in the dashboard. The daemon
#    reconnects and the board switches from all-on to live status.
```

The golden rule: **only one program can hold the serial port.** Whatever is
driving the LEDs (the menu bar daemon, or a foreground `clawlight led`) and
`espflash` both want it, so LED output must be stopped before every flash and
restarted after.

## Full procedure

### 1. Hardware check

- Use a **data** USB-C cable, not a charge-only one. If unsure, use the
  cable that last flashed successfully.
- Plug the XIAO's single USB-C port into your Mac. It's wired straight to the
  native USB-Serial-JTAG — there's only one port and no wrong choice.
- Confirm the board enumerated:
  ```bash
  ls /dev/cu.usbmodem*
  ```
  You should see something like `/dev/cu.usbmodem101`. No device → it's a
  cable or port problem, not software (see Troubleshooting).

### 2. Free the serial port

The menu bar daemon drives the LEDs whenever they're enabled, and it keeps the
serial port open. Turn LEDs off so it lets go:

- **In the dashboard:** press **`l`**. This flips `led_enabled` to `false` in
  `~/.claude/clawlight/config.json`; the daemon stops touching the port within
  ~2 s. (You can also edit that file directly if the dashboard isn't handy.)
- **If you're running the foreground driver** (`clawlight led`), just **Ctrl-C**
  it.

Verify nothing is holding the port:

```bash
lsof /dev/cu.usbmodem*        # should print nothing
```

If LEDs are off but `lsof` still shows a holder, it's a stray foreground
`clawlight led` — kill it:

```bash
pkill -f "clawlight led"
```

> You normally don't need to stop the whole menu bar daemon — toggling LEDs
> off is enough, and it keeps the menu bar icon alive. If you do want to stop
> it entirely (it's a launchd LaunchAgent), unload it for the duration:
> ```bash
> launchctl bootout gui/$(id -u)/io.roush.clawlight.menubar
> ```
> Remember to `launchctl bootstrap` it back (or just log out/in) afterwards.

### 3. Build and flash

```bash
cd ~/Developer/clawlight-firmware
cargo run --release
```

`cargo run` invokes `espflash flash --monitor --chip esp32c6` via the
runner in `.cargo/config.toml`. Expect:

- a build (instant if nothing changed),
- chip detection + flash (unchanged segments are skipped — fast),
- `Flashing has completed!`,
- the serial monitor opens, showing the ESP-IDF bootloader's `I (…) boot:`
  lines. **The firmware itself prints nothing** — it never writes to USB
  by design. Silence after the boot log is normal, not a hang.

On the board you'll see the boot lamp test: all three LEDs on together for
2 seconds, then they hold all-on until the daemon connects.

Press **Ctrl-C** to exit the monitor. The firmware keeps running.

> **XIAO ESP32-C6:** its auto-reset into download mode is unreliable, so
> the flash often **times out** (`espflash::timeout`) on the first try.
> Enter the bootloader by hand — see [Entering the bootloader
> manually](#entering-the-bootloader-manually) — then flash.

### 4. Re-enable LEDs

Press **`l`** again in the dashboard (or set `led_enabled` back to `true`). The
menu bar daemon auto-detects the board, prints `LED: connected to
/dev/cu.usbmodem…`, and the board's all-on collapses to the live status color.

## Flash without re-building (optional)

If you already have a release binary and just want to reflash:

```bash
espflash flash --chip esp32c6 --port /dev/cu.usbmodem101 \
  target/riscv32imac-unknown-none-elf/release/clawlight-firmware
```

Pass `--port` explicitly when more than one serial device is present —
otherwise espflash shows an interactive picker (and fails with "not a
terminal" if run non-interactively).

## Entering the bootloader manually

Flashing needs the chip in ROM download mode. The **XIAO ESP32-C6's
auto-reset is unreliable** and often doesn't enter it on its own when espflash
connects, so force it:

1. **Press and hold BOOT.**
2. While holding BOOT, **tap RESET** once. (No RESET handy, or it's
   fiddly? Unplug/replug the USB-C cable while holding BOOT instead.)
3. **Release BOOT.**

A serial monitor on the port will print the ROM banner and
`waiting for download` — that's the confirmation you're in:

```
rst:0x15 (USB_UART_HPSYS),boot:0x16 (DOWNLOAD(USB/UART0/SDIO...))
...
waiting for download
```

The chip stays in download mode until the next reset, so there's no rush.
Close any monitor holding the port, then run the flash command — espflash
connects instantly (no second BOOT dance). After flashing, espflash
resets the chip back into the app automatically.

> The `Saved PC: ... core::mem::replace` line in that banner is just the
> reset diagnostic showing where the old firmware was — not an error.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `No serial ports could be detected` | Charge-only cable, or board not enumerating. `ls /dev/cu.usbmodem*` shows nothing; USB bus has no new device. | Swap to a known-good **data** cable. Check the power LED: lit but no serial = charge-only cable; dark = no power (dead cable/port). Try another Mac port. |
| `Device or resource busy` | Another program holds the port — almost always the clawlight LED driver (menu bar daemon or a foreground `clawlight led`). | Turn LEDs off in the dashboard (`l`), confirm with `lsof /dev/cu.usbmodem*`, reflash. A stray foreground driver: `pkill -f "clawlight led"`. |
| `espflash::timeout` / "Timeout while running command" | Port exists but the chip never answered the sync — it isn't in download mode. Common on the XIAO (weak auto-reset). | [Enter the bootloader manually](#entering-the-bootloader-manually) (hold BOOT, tap RESET, release), then reflash. If multiple boards are plugged in, espflash may be hitting the wrong one — leave only the target attached. |
| `not a terminal` (dialoguer error) | espflash needs to show a port picker but isn't attached to a TTY. | Pass `--port /dev/cu.usbmodem101` explicitly. |
| Picked `tty.*` and it hangs on open | `tty.*` (dial-in) blocks waiting for carrier-detect. | Always use the `cu.*` (callout) device on macOS. |
| Flash OK but USB port dead afterward (bad firmware) | Firmware crashed early or repurposed the USB pins (GPIO12/13). | Recovery: hold **BOOT**, tap **RST**, release BOOT → ROM bootloader, then reflash. |

## Why the daemon and flasher conflict

A serial port is an exclusive resource — the OS lets exactly one process
open `/dev/cu.usbmodem101` at a time. The clawlight LED driver opens it to
stream status bytes and keeps it open. `espflash` needs it to talk to the
bootloader. So every flash is: **stop LED output → flash → restart LED
output.** This runbook exists because that ordering isn't obvious until it
bites you.
