# Serial OTA — updating firmware over the existing USB link

After a one-time bootstrap flash, new firmware is pushed to the board over the
**same USB-Serial-JTAG link the clawlight daemon already uses for LEDs** — no
cable reflash, no BOOT/RST dance, no port handoff. This documents the on-wire
protocol (defined by [`../src/main.rs`](../src/main.rs)), the partition layout
([`../partitions.csv`](../partitions.csv)), and the safety model. The host side
lives in the clawlight-cli `update` command.

## Why this works without a reflash

The board carries the standard ESP-IDF **two-slot OTA layout**: the running
image lives in one app slot while a new image is written to the *other*, then
the `otadata` partition is flipped so the bootloader runs the new slot on the
next boot. Because you always write the **inactive** slot, the live firmware is
never at risk during a transfer.

```
factory   ← bootstrap image (first cable flash lands here)
ota_0     ← first OTA lands here, then it ping-pongs…
ota_1     ← …with ota_1
otadata   ← which slot boots next (flipped on a successful update)
```

## The one-time bootstrap

Today's single-app boards have no second slot, so enabling OTA needs **one**
final cable flash with the new partition table:

```bash
cd ~/Developer/clawlight-firmware
cargo run --release        # installs partitions.csv + image into `factory`
```

(See the [flashing runbook](esp32-flashing-runbook.md) for freeing the port
first.) After this, every further update is a serial push — no cable.

## Wire protocol

115200 baud, same port as the LED protocol. An update is triggered by a normal
newline-terminated line; the firmware distinguishes it by the `OTA:` prefix
(the LED protocol never emits one, and old firmware simply ignores it). Update
mode is the **only** time the board transmits — one ASCII byte + `\n` per
reply — which is safe because the host is actively reading during a transfer.

```
host → board   OTA:<len>:<crc32>\n    trigger: image length (decimal bytes)
                                       + CRC-32 (hex, zlib/IEEE) of the image
board → host   K\n                     ready: inactive slot found and ≥ len
   ── repeat, 4096-byte blocks (last may be short) ──
host → board   <block bytes>           one block, then wait for the ack
board → host   K\n                     block written to flash; send the next
   ───────────────────────────────────────────────
board → host   D\n   then reboots      whole-image CRC matched → slot activated
board → host   E\n                     abort (bad trigger / no OTA layout / slot
                                        too small / stalled / write fail / CRC
                                        mismatch); running image untouched
```

Stop-and-wait (one ack per block) gives natural flow control — the host never
outruns the flash write, and either side can abort cleanly with no partial
state applied. A transfer that stalls for 3 s (`OTA_STALL_MS`) is abandoned and
the board drops back to normal LED operation.

### Host pseudocode

```
img   = read(firmware.bin)
crc   = crc32(img)                       # standard zlib CRC-32
send(f"OTA:{len(img)}:{crc:08x}\n")
expect("K")
for block in chunks(img, 4096):
    send(block); expect("K")
expect("D")                              # board reboots into the new image
# then just let the existing reconnect logic re-open the port
```

## Safety model — why this can't brick the board

Three independent nets, weakest to strongest:

1. **Inactive-slot writes.** The running image is never touched. A transfer
   that dies mid-stream leaves a partial image in the *other* slot that is
   never activated — the next boot still runs the good image.
2. **Verify-before-activate.** The slot is activated only after the entire
   image matches the trigger's CRC-32. A corrupt or truncated transfer answers
   `E\n` and changes nothing.
3. **ROM download mode.** The ultimate backstop for a logically-broken but
   CRC-valid image (compiles and flashes, but crashes): hold **BOOT**, tap
   **RST** → ROM bootloader → reflash, exactly as the recovery runbook says.

> **Auto-rollback is not yet active.** True automatic rollback — where the
> bootloader reverts to the previous slot if the new image never confirms
> itself healthy — requires a bootloader *built with rollback support*. The
> stock bootloader espflash ships does **not** have it. The firmware already
> does its part (marks a new slot `New`, and confirms the running slot `Valid`
> once the daemon reconnects), so enabling real rollback later is a
> bootloader-build change, not a firmware-logic change. Until then, net #3 is
> the recovery path for a bad-but-valid image.
