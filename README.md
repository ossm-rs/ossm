# OSSM-rs

An alternative firmware for OSSM written in rust.

## Quick Links

- [Why](#why)
- [Supported hardware](#supported-hardware)
- [Installing firmware](#installing-firmware)
- [Status LED colors](#status-led-colors)
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

- [OSSM Reference Board](https://github.com/KinkyMakers/OSSM-hardware) (ESP32, step/dir)
  The original OSSM PCB with step/dir motor control and current-sensing homing.

- [Industrial ESP32-S3-RS485-CAN](https://www.waveshare.com/esp32-s3-rs485-can.htm) by WaveShare

- Seeed Studio's Xiao ESP32-S3

  You'll need:
  - [XIAO ESP32-S3](https://www.seeedstudio.com/XIAO-ESP32S3-p-5627.html) MCU
  - [RS384 Breakout board](https://www.seeedstudio.com/RS485-Breakout-Board-for-XIAO-p-6306.html) for the XIAO ESP32-S3
  - A power supply for the motor

Motors:

- 57AIM series motor (RS485/Modbus or step/dir)

## Installing firmware

For now, firmware must be built and flashed manually from source. See [Set up your environment](#set-up-your-environment) for instructions.

For the v1 release, pre-built binaries will be published on each tagged release. Longer term, a web flasher is planned so you can flash directly from your browser without any toolchain setup.

## Status LED colors

On boards with a configured RGB status LED, such as the OSSM Alt Edition, the LED shows a steady color:

| Color | Meaning |
| --- | --- |
| Dim white | Booting — initial indication while the machine is starting up. |
| Blue | Idle — stopped and waiting for a command. |
| Purple | Homing |
| Orange | Stopping — decelerating the machine. |
| Green | Ready, moving, or playing a pattern. |
| Yellow | Paused |
| Red | An error has stopped the machine. Restart it to try again. |
| Blinking red (green on WaveShare) | A known problem has stopped the machine. Count the blinks, then restart it. |

Blinking red (green on WaveShare) LED repeats a group of blinks separated by a longer pause:

| Blinks | Meaning |
| --- | --- |
| 1 | The motor did not respond during startup. Check motor power and wiring, then restart. |
| 2 | The motor was configured for this firmware. Power cycle the motor and the board. |
| 3 | The board could not set up its hardware. Check that the firmware matches the board, then restart. |

On boards with a single-color user LED, such as the Seeed XIAO ESP32S3, the LED stays off during normal operation. It blinks the same codes, and stays on steadily after an error.

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
- no_std compatible - runs on bare-metal embedded targets like ESP32 and ESP32-S3

## Anatomy

The project is split into several layers, each with a clear responsibility. Here's how they fit together from the top down:

```
┌──────────────────────────────────────────┐
│                Firmware                  │
│   (A platform for each target + modes)   │
├──────────────────────────────────────────┤
│                  Ossm                    │
│    (Runtime: boot order and modes)       │
├───────────────────┬──────────────────────┤
│     Patterns      │     Controller       │
│     (Feature)     │      (Feature)       │
├───────────────────┴──────────────────────┤
│             Motion channels              │
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

### Ossm (Runtime)

`Ossm` is where firmware starts. A firmware registers the modes the machine can switch between, then hands `Ossm` a `Platform` for its target:

```rust
Ossm::new(Esp32s3::new(spawner, config))
    .mode(OssmMode::Patterns, patterns)
    .start_in(OssmMode::Patterns)
    .run()
    .await
```

`run()` owns the boot order: it sets up the hardware through the platform, starts the motion controller, starts the indicator and remotes, then runs the modes. Each mode is an async function whose arguments are the only capabilities it gets, such as permission to move the motor. An idle mode is always available.

### Motion channels (Public Control)

The motion channels are how features drive the machine. On their own they don't do anything - the `MotionSender` exposes methods like `enable()`, `disable()`, `home()`, `pause()`, `resume()`, `begin_motion()`, `update_motion()`, and `await_motion()` that forward commands over a lock-free channel to the motion controller. It's up to features (like the pattern engine or a controller) to call these methods and drive the machine.

### Motion Controller

The motion controller is the real-time engine that runs on a high-priority interrupt executor. It receives commands from the motion channels, manages a state machine (Disabled → Enabled → Ready → Moving), and uses Ruckig for jerk-limited trajectory planning. Each update cycle it converts the planned trajectory into motor step commands. It also handles mechanical configuration (pulley teeth, belt pitch, travel limits) and motion limits (max velocity, acceleration, jerk).

### Boards

A board crate handles the hardware-specific setup for a particular PCB - initialising UART peripherals, configuring GPIO pins, and providing any wrappers needed by the hardware (such as an RS485 half-duplex driver that toggles a direction-enable pin). The board produces a configured motor instance ready for use by the motion controller.

### Motors

The `Motor` trait defines a hardware-agnostic interface that the motion controller programs against: `enable`, `disable`, `home`, `set_absolute_position`, `set_speed`, etc. Concrete driver crates (e.g. the M57AIM driver) implement this trait using the appropriate protocol - in the case of the 57AIM series, Modbus RTU over UART. Adding support for a new motor means writing a new implementation of the `Motor` trait.

### Firmware

A firmware crate is the final integration point that ties a specific board and motor together into a flashable binary. It implements `Platform` for its target: setting up the board and motor, running the motion controller on a real-time executor, and starting the indicator and radios. Its binaries pick the modes and hand the platform to `Ossm`. Each supported chip gets its own firmware crate.

### Features

Features are optional higher-level capabilities built on top of the core motion control. The pattern engine is the primary feature today - it defines a `Pattern` trait whose implementations describe repeating motion sequences (simple stroking, stop-and-go, teasing/pounding, etc.). Patterns read live user input (depth, stroke, velocity, sensation) and translate it into motion commands via the motion channels. New patterns can be added by implementing the `Pattern` trait. Future features may include user remotes, mobile apps, or video game integrations - all built on top of the same public control layer.

## Contributing

### Set up your environment

#### Option 1: Dev container (recommended)

1. Install [Docker](https://www.docker.com/).
2. Open the repo in [VS Code](https://code.visualstudio.com/) with the [Dev Containers extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers), or in [Zed](https://zed.dev/docs/dev-containers), and accept the prompt to reopen it in the container.
3. Once the first build finishes, check the setup with `just doctor`.

The container can build firmware and run the web simulator, but it can't reach the board's USB port. To flash, install [espflash](https://github.com/esp-rs/espflash) on the host and flash the built image from there:

```sh
espflash flash --monitor ossm/firmware/esp32s3/target/xtensa-esp32s3-none-elf/release/ossm-alt
```

#### Option 2: On the host

1. Install [Rust](https://rustup.rs/).
2. Install the ESP toolchain at the version CI uses:

   ```sh
   cargo install espup --locked
   espup install --toolchain-version "$(cat .esp-toolchain-version)"
   ```

3. Install the build tools: `cargo install just espflash --locked`, plus [jq](https://jqlang.org/download/) from your package manager.
4. Optional, for the web simulator: install the simulator tools listed in [So You Want to Add a Pattern](docs/so-you-want-to-add-a-pattern.md#setting-up-your-environment).
5. Check the setup with `just doctor`.

If you use Nix, `nix develop` provides everything except the ESP toolchain. Inside it, run `espup install --export-file "$HOME/export-esp.sh"` once.

#### Build and flash

```sh
just                       # list all recipes
just build ossm-alt        # build a firmware variant
just flash ossm-alt        # build, flash and monitor (host only)
just web-tools             # run the web simulator
```

VS Code and Zed have tasks for the common recipes. Run **Tasks: Run Task** in VS Code or **task: spawn** in Zed.

#### Configuring rust-analyzer

rust-analyzer can only analyze one target at a time, so tell it which firmware to use, otherwise it reports false errors:

```sh
just focus esp32s3   # or esp32
```

Then restart rust-analyzer. This regenerates the editor settings from their templates, so put lasting changes in the templates. The dev container runs `just focus esp32s3` when it's first created.

## Shoulders of giants

This work would not be possible without the open source community, we thank you. Specifically help from the following made this project possible.

- [@orange_gem](https://github.com/orange-gem/) - Created the original [ossm-rs](https://github.com/orange-gem/ossm-rs) that this project is based on, proving that a Rust implementation of the OSSM firmware was viable.
- [@jollydodo](https://github.com/jollydodo) - Designed the [OSSM Alt Edition](https://github.com/jollydodo/OSSM-ALT-Edition) board that made Rust-based OSSM firmware possible in the first place.
- The R&D team - For pioneering the idea of an open source project like this.
