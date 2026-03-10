# OSSM-rs

An alternative firmware for OSSM written in rust.

## Quick Links

- [Why](#why)
- [Supported hardware](#supported-hardware)
- [Installing firmware](#installing-firmware)
- [Safety and disclaimers](#safety-and-disclaimers)
- [Features](#features)
  - [Under the hood](#under-the-hood)
- [Anatomy](#anatomy)
- [Contributing](#contributing)
- [Shoulders of giants](#shoulders-of-giants)

## Why

While the original OSSM firmware does the job, rebuilding in Rust gives us a stronger foundation to grow from:

- **Safety** - Rust's type system and ownership model catch entire classes of bugs at compile time. For a project like this, that matters.
- **Modularity** - The codebase is split into small, focused crates (core library, motor drivers, board support, features) that can be developed and tested independently.
- **Testability** - Trait-based abstractions mean the motion controller, patterns, and drivers can all be tested without real hardware.
- **Portability** - The core library is `no_std` and platform-agnostic. It can target ESP32, other microcontrollers, or even compile to WebAssembly for simulation and tooling in the browser.
- **Telemetry** - Using RS485 for motor communication lets us read back real-time data like position, current draw, voltage, and temperature. This telemetry opens the door to smarter safety checks — detecting stalls, overcurrent, or overheating before they become a problem.

Together, these make the project easier to understand, safer to modify, and more welcoming to contributions from everyone - whether you're writing a new pattern, adding a motor driver, or building a web-based remote.

## Supported hardware

Boards:

- [OSSM-ALT-Edition](https://github.com/jollydodo/OSSM-ALT-Edition) by [@jollydodo](https://github.com/jollydodo).

  A cheap, compact, powerful s3 board with built in 28v USB-PD.

- Industrial ESP32-S3-RS485-CAN by WaveShare

  Coming soon.

Motors:

- 57AIM series motor

## Installing firmware

For now, firmware must be built and flashed manually from source. See [Set up your environment](#set-up-your-environment) for instructions.

For the v1 release, pre-built binaries will be published on each tagged release. Longer term, a web flasher is planned so you can flash directly from your browser without any toolchain setup.

## Safety and disclaimers

While every precaution has been taken to make this software as safe as possible, the authors and contributors accept no responsibility or liability for any damage, injury, or harm that may result from the use or misuse of this software. This software is provided "as is", without warranty of any kind, express or implied. Use at your own risk.

## Features

- Smooth, jerk-limited motion profiles for safe predictable operation
- Hardware homing with automatic position reset
- Configurable motion limits (velocity, acceleration, jerk) per machine
- Position clamping to prevent over-travel beyond mechanical bounds
- Pattern engine with live user input (depth, stroke, velocity, sensation)
- 6 built-in patterns: Simple, Deeper, Half'n'Half, Stop'n'Go, Teasing Pounding, and Torque
- Extensible - add custom patterns, motors, or boards by implementing a trait

### Under the hood

- Real-time motion control on a high-priority interrupt executor with fixed time-step updates
- Jerk-limited trajectory planning powered by Ruckig (S-curve acceleration profiles)
- Simple fractional API - positions and velocities expressed as 0.0-1.0, decoupled from machine geometry
- Motion state machine with safe transitions (Disabled → Enabled → Ready → Moving)
- Trait-based hardware abstraction for motors and boards
- Optional motor telemetry (position, speed, current, voltage, temperature)
- Fully async, lock-free architecture built on Embassy channels and signals
- no_std compatible - runs on bare-metal embedded targets like ESP32-S3

## Anatomy

The project is split into several layers, each with a clear responsibility. Here's how they fit together from the top down:

```
┌──────────────────────────────────────────┐
│                Firmware                  │
│    (Assembles everything for a target)   │
├───────────────────┬──────────────────────┤
│     Patterns      │     Controller       │
│     (Feature)     │      (Feature)       │
├───────────────────┴──────────────────────┤
│                  Ossm                    │
│            (Public Control)              │
├──────────────────────────────────────────┤
│            Motion Controller             │
│      (Trajectory planning + state)       │
├────────────────────┬─────────────────────┤
│       Board        │    Motor Driver     │
│   (Pin/peripheral  │   (Hardware comms,  │
│       setup)       │    e.g. Modbus)     │
└────────────────────┴─────────────────────┘
```

### Ossm (Public Control)

`Ossm` is the public-facing API that features and application code interact with. On its own it doesn't do anything - it simply exposes methods like `enable()`, `disable()`, `home()`, `move_to()`, and `set_speed()` that accept fractional values (0.0-1.0) and forwards them over a lock-free channel to the motion controller. It's up to features (like the pattern engine or a controller) to call these methods and drive the machine.

### Motion Controller

The motion controller is the real-time engine that runs on a high-priority interrupt executor. It receives commands from the `Ossm` channel, manages a state machine (Disabled → Enabled → Ready → Moving), and uses Ruckig for jerk-limited trajectory planning. Each update cycle it converts the planned trajectory into motor step commands. It also handles mechanical configuration (pulley teeth, belt pitch, travel limits) and motion limits (max velocity, acceleration, jerk).

### Boards

A board crate handles the hardware-specific setup for a particular PCB - initialising UART peripherals, configuring GPIO pins, and providing any wrappers needed by the hardware (such as an RS485 half-duplex driver that toggles a direction-enable pin). The board produces a configured motor instance ready for use by the motion controller.

### Motors

The `Motor` trait defines a hardware-agnostic interface that the motion controller programs against: `enable`, `disable`, `home`, `set_absolute_position`, `set_speed`, etc. Concrete driver crates (e.g. the M57AIM driver) implement this trait using the appropriate protocol - in the case of the 57AIM series, Modbus RTU over UART. Adding support for a new motor means writing a new implementation of the `Motor` trait.

### Firmware

A firmware crate is the final integration point that ties a specific board and motor together into a flashable binary. It initialises the board, creates the `Ossm` and `MotionController` pair, spawns the motion task on a real-time executor, runs the homing sequence, and then hands control to a pattern or application loop. Each supported hardware combination gets its own firmware crate.

### Features

Features are optional higher-level capabilities built on top of the core motion control. The pattern engine is the primary feature today - it defines a `Pattern` trait whose implementations describe repeating motion sequences (simple stroking, stop-and-go, teasing/pounding, etc.). Patterns read live user input (depth, stroke, velocity, sensation) and translate it into motion commands via the `Ossm` API. New patterns can be added by implementing the `Pattern` trait. Future features may include user remotes, mobile apps, or video game integrations - all built on top of the same public control layer.

## Contributing

### Set up your environment

#### On a host

1. **Install Rust** via [rustup](https://rustup.rs/):

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Install the ESP Rust toolchain** ([espup](https://github.com/esp-rs/espup)):

   ```sh
   cargo install espup
   espup install
   ```

   This adds the `xtensa` target and the `+esp` toolchain needed to compile for ESP32-S3.

3. **Install [just](https://github.com/casey/just)** (a command runner used for build recipes):

   ```sh
   cargo install just
   ```

   Optionally, enable shell completions for `just` so you can tab-complete recipe names:

   ```sh
   # zsh (add to ~/.zshrc)
   eval "$(just --completions zsh)"

   # bash (add to ~/.bashrc)
   eval "$(just --completions bash)"

   # fish (add to ~/.config/fish/config.fish)
   just --completions fish | source
   ```

4. **Install [espflash](https://github.com/esp-rs/espflash)** (for flashing firmware to the board):

   ```sh
   cargo install espflash
   ```

5. You should now be able to build and flash:

   ```sh
   just build
   just flash
   ```

#### Configuring rust-analyzer

This project targets multiple architectures (ESP32, ESP32-S3, WASM), each with its own Rust target triple and cargo features. Since rust-analyzer can only analyze one target at a time, it needs to be told which one to use - otherwise it defaults to your host platform and will report false errors for embedded or WASM code.

The `just focus` command links a firmware's config tomls to the workspace root to configure the correct target and features:

```sh
just focus ossm-alt
just focus wasm
```

After running this, you may need to restart rust-analyzer (or reload your editor) to pick up the new settings. You only need to re-run it when switching to a different target.

> Note: In unix this uses a symlink, meaning if either file changes, both kept in sync. Windows has permission issues with symlinks, and so it performs a full copy instead. Edits to the root level configs will not be persisted, and can fall out of sync.

#### In a dev container

The dev container comes with Rust and all required tooling pre-installed, so you can start building straight away.

However, to flash firmware you need to expose the UART device to the container. Add a `runArgs` entry to `.devcontainer/devcontainer.json` with the path to your serial device:

```json
{
  "name": "Rust",
  "image": "mcr.microsoft.com/devcontainers/rust:2-1-trixie",

  // Expose the UART device to the container
  "runArgs": ["--device=/dev/ttyUSB0"]
}
```

Replace `/dev/ttyUSB0` with the actual path to your device (e.g. `/dev/ttyACM0` on some systems, or `/dev/cu.usbserial-*` on macOS).

> **Note:** If the UART device is disconnected and reconnected while the container is running, the container will lose access to it. You will need to restart the container for the device to become available again.

## Shoulders of giants

This work would not be possible without the open source community, we thank you. Specifically help from the following made this project possible.

- [@orange_gem](https://github.com/orange-gem/) - Created the original [ossm-rs](https://github.com/orange-gem/ossm-rs) that this project is based on, proving that a Rust implementation of the OSSM firmware was viable.
- [@jollydodo](https://github.com/jollydodo) - Designed the [OSSM Alt Edition](https://github.com/jollydodo/OSSM-ALT-Edition) board that made Rust-based OSSM firmware possible in the first place.
- The R&D team - For pioneering the idea of an open source project like this.
