<p align="center">
  <img src="assets/icon.png" width="128" alt="Serial Debug icon">
</p>

<h1 align="center">Serial Debug</h1>

<p align="center">
  A cross-platform serial port debugging tool built with Rust and <a href="https://www.gpui.rs/">GPUI</a>.
</p>

## Features

- **Full port configuration** — port, baud rate (presets or any custom value), data bits (5–8), stop bits (1/2), parity (none/odd/even) and flow control (none/software/hardware).
- **ASCII and HEX, both ways** — send and receive in either format, switchable at any time. The log keeps the raw bytes, so switching the display mode re-renders the whole history.
- **Timestamps** — every sent and received message is shown with an `HH:MM:SS.mmm` timestamp (can be toggled).
- **Send suffixes** — built-in `\r`, `\n`, `\r\n` and "none", plus your own custom suffixes. Custom suffixes use escape syntax (`\r \n \t \0 \\ \xNN`), e.g. `\x0D\x0A\x00`, and are saved between sessions.
- **Receive framing** — bytes arriving close together are merged into a single log entry (a short idle gap ends a frame).
- **Terminal mode** — switch to an interactive terminal: keystrokes (including arrows, Ctrl+key, Ctrl+Shift+V paste) go straight to the port, and received text is rendered with ``/`
`/backspace handling and ANSI escape sequences filtered. Optional local echo.
- **Modbus RTU master** — a third mode with the eight common function codes (01, 02, 03, 04, 05, 06, 0F, 10), automatic CRC-16 and broadcast support. Requests can be sent once or polled at a configurable interval; register values can optionally be decoded as u16/i16/u32/i32/f32 with ABCD/CDAB/BADC/DCBA byte/word order. Parsed responses are shown in a dedicated table while the raw TX/RX frames are written to the HEX log.
- **Convenience** — auto-scroll, clear, copy the whole log to the clipboard, TX/RX byte counters, and persisted settings.

Only serial port connections are supported for now: plain serial plus Modbus RTU over serial. There is no Modbus TCP or other network transport.

## Download

Prebuilt binaries for Windows, Linux and macOS (Intel and Apple silicon) are built by GitHub Actions. Download them from the **Actions** tab (build artifacts) or, once a version is tagged, from **Releases**.

## Usage

1. Pick a serial port (use **Refresh** if it was plugged in after launch) and set the parameters.
2. Click **Open**. Parameters are locked while the port is open.
3. Choose the send format (ASCII / HEX) and a suffix, type a message and press **Enter** or click **Send**.
   - In HEX mode, input such as `AA 55 0x01, ff` is accepted (spaces, commas and `0x` prefixes are ignored).
4. Use the **Receive display** switch above the log to view incoming data as ASCII or HEX.

To add a custom suffix, click **Add suffix**, enter a name and its content in escape syntax, and confirm.

### Modbus RTU

1. Switch the mode selector to **Modbus** and configure the serial port as usual (most Modbus RTU devices use 8N1 or 8E1 with 9600, 19200 or 115200 baud).
2. Pick the slave address (0 is broadcast for write requests) and a function code, then fill in the start address plus the quantity — or, for the write-multiple functions, the values:
   - coils for 0F: a `0`/`1` sequence such as `1 0 1 1`;
   - registers for 10: decimal or `0x` values such as `0 100 0x1F`.
3. Click **Send**. The parsed result (values, exception code or error message) is added to the results table, while the complete request and response frames are appended to the log in HEX. The CRC is calculated and checked automatically.
4. Toggle **Poll** and set an interval in milliseconds to repeat the request; toggle it again to stop. Responses time out after one second.
5. For 03/04 reads, choose a decoding format. The 32-bit formats combine two registers and let you select the byte/word order (ABCD, CDAB, BADC or DCBA).

Settings for the Modbus form (including the last request and its decoding options) are persisted with the rest of the configuration.

Settings are stored in `%APPDATA%\serial-debug\config.json` on Windows (the `APPDATA` environment variable is used; if it is not set, settings are not persisted).

## Building from source

Requires a recent stable Rust toolchain.

```sh
cargo run --release
```

Platform notes (GPUI system requirements):

- **Windows** — MSVC toolchain and the Windows SDK.
- **macOS** — Xcode with the Metal toolchain (`xcode-select --install`; newer Xcode versions may need `xcodebuild -downloadComponent MetalToolchain`).
- **Linux** — development packages for udev, fontconfig, Wayland/X11 (xkbcommon, xcb) and Vulkan, for example on Debian/Ubuntu:

  ```sh
  sudo apt-get install build-essential pkg-config cmake clang \
    libudev-dev libasound2-dev libfontconfig-dev libfreetype-dev \
    libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libxcb1-dev libx11-xcb-dev libvulkan1 libvulkan-dev libssl-dev libzstd-dev
  ```

  To open serial ports without root, add your user to the `dialout` group.

Run the unit tests with `cargo test`.

> **Note:** `Cargo.lock` pins `libc` to 0.2.188 because `xattr 0.2.3` (a transitive dependency of GPUI) does not compile against `libc` 0.2.190 or newer on Linux. Avoid updating it until GPUI moves to a newer `xattr`.

## Status

Developed and tested mainly on Windows. Linux and macOS builds are produced by CI and compile successfully, but have had little hands-on testing. macOS binaries are unsigned, so Gatekeeper may ask you to open them via right-click → Open.

## License

[MIT](LICENSE)
