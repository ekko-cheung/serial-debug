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
- **Convenience** — auto-scroll, clear, copy the whole log to the clipboard, TX/RX byte counters, and persisted settings.

Only the standard serial port protocol is supported for now.

## Download

Prebuilt binaries for Windows, Linux and macOS (Intel and Apple silicon) are built by GitHub Actions. Download them from the **Actions** tab (build artifacts) or, once a version is tagged, from **Releases**.

## Usage

1. Pick a serial port (use **Refresh** if it was plugged in after launch) and set the parameters.
2. Click **Open**. Parameters are locked while the port is open.
3. Choose the send format (ASCII / HEX) and a suffix, type a message and press **Enter** or click **Send**.
   - In HEX mode, input such as `AA 55 0x01, ff` is accepted (spaces, commas and `0x` prefixes are ignored).
4. Use the **Receive display** switch above the log to view incoming data as ASCII or HEX.

To add a custom suffix, click **Add suffix**, enter a name and its content in escape syntax, and confirm.

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
