# So You Want to Add a Pattern

This guide walks you through setting up your environment, running the simulator, and adding a new pattern to the pattern engine.

## Setting Up Your Environment

The [dev container](https://github.com/ossm-rs/ossm#option-1-dev-container-recommended) described in the README has everything you need. If you use it, skip to [Running the Simulator](#running-the-simulator).

To work on the host, install the following. Patterns don't need the ESP toolchain.

| Tool                                                       | Purpose                               |
| ---------------------------------------------------------- | ------------------------------------- |
| [Rust](https://rustup.rs/)                                 | Compiler toolchain                    |
| [wasm-bindgen-cli](https://crates.io/crates/wasm-bindgen-cli) (`cargo install wasm-bindgen-cli --locked --version "$(bash scripts/wasm-bindgen-version.sh)"`, must match the bindings' `Cargo.lock`) | Generates JS bindings for the WASM module |
| [binaryen](https://github.com/WebAssembly/binaryen) (`wasm-opt`) | Optimises the WASM output |
| [Node.js](https://nodejs.org/en/download) (v24 LTS)        | Runs the simulator dev server         |
| [pnpm](https://pnpm.io/installation#using-corepack)        | Package manager for the simulator app |
| [just](https://just.systems/man/en/packages.html)          | Command runner                        |

Then add the WASM target and install simulator dependencies:

```sh
rustup target add wasm32-unknown-unknown
pnpm install --dir apps/web-tools
```

## Running the Simulator

```sh
just web-tools
```

This does two things in sequence:

1. **`build-wasm`** - compiles the web simulator and trajectory recorder bindings (which include the pattern engine) to WebAssembly using `cargo build`, then generates JS bindings with `wasm-bindgen` and optimises the output with `wasm-opt`.
2. **`pnpm dev`** - starts the Vite dev server in `apps/web-tools/` with `--host` so you can access it from other devices on your network.

The simulator renders a 3D model of the OSSM and runs your patterns in the browser. Change a pattern and the wasm will recompile and the browser will reload.

### Cloudflare Worker Functions

The web tools app includes Cloudflare Worker functions (e.g. the GitHub asset proxy used by the flasher) in `src/api/`. The Cloudflare Vite plugin runs these in the local `workerd` runtime during `pnpm dev`, so both the SPA and `/api/*` routes work with HMR out of the box.

Cloudflare Worker functions should be used sparingly and only when entirely necessary (e.g. proxying requests to avoid CORS issues). Prefer doing work client-side wherever possible.

## Adding a New Pattern

### 1. Create the pattern file

Add a new file in `ossm/crates/pattern-engine/src/patterns/`. For example, `my_pattern.rs`:

```rust
use embedded_hal_async::delay::DelayNs;

use crate::pattern::{Pattern, PatternCtx};

pub struct MyPattern;

impl Pattern for MyPattern {
    const NAME: &'static str = "My Pattern";
    const DESCRIPTION: &'static str = "Describe what it does";

    async fn run(&mut self, ctx: &mut PatternCtx<'_, impl DelayNs>) -> Result<(), ossm::Cancelled> {
        loop {
            ctx.motion().position(1.0).send().await?;
            ctx.motion().position(0.0).send().await?;
        }
    }
}
```

### 2. Re-export from `patterns/mod.rs`

```rust
mod my_pattern;
pub use my_pattern::MyPattern;
```

### 3. Register in `lib.rs`

Add an entry to the `define_patterns!` invocation:

```rust
define_patterns! {
    Simple(Simple),
    Deeper(Deeper),
    // ...existing patterns...
    MyPattern(MyPattern),  // <-- add this
}
```

Also add it to the `use crate::patterns::{...}` import in `lib.rs`.

### 4. Test it

```sh
just web-tools
```

Open the simulator in your browser, select your pattern from the dropdown, and verify it behaves as expected.

## Pattern API Reference

Inside `run()`, you interact with the motor through `PatternCtx`. The API is split into three areas: **motion**, **sensation**, and **delay**.

### Motion

Motion commands use a builder pattern: start with `ctx.motion()`, set a `.position()`, optionally chain `.speed()`, `.jerk()` and/or `.torque()`, then `.send().await?` to execute.

Every `.send().await?` is a cancellation point - when the engine needs to stop the pattern, the `?` propagates `Err(Cancelled)` and exits `run()` cleanly. Always use `?` after `.send().await`.

| Method            | Description                                                                                                          |
| ----------------- | -------------------------------------------------------------------------------------------------------------------- |
| `.position(f)`    | **Required.** Target position as a fraction of stroke range (0.0 = shallowest, 1.0 = deepest). Clamped to 0.0–1.0.   |
| `.speed(factor)`  | Velocity multiplier relative to the user's chosen speed. Default is `1.0`. `0.5` = half speed. Clamped to 0.0–1.0. |
| `.jerk(factor)`   | Jerk factor from 0.0 (smooth) to 1.0 (choppy). Default is `0.5`. Clamped to 0.0–1.0.                              |
| `.torque(factor)` | Torque limit as a factor from 0.0–1.0. Omitting uses the motor's default torque.                                     |
| `.send().await?`  | Executes the command. Blocks until the motor reaches the target position.                                            |

```rust
// Simple full-range stroke
ctx.motion().position(1.0).send().await?;
ctx.motion().position(0.0).send().await?;

// Move at half speed
ctx.motion().position(1.0).speed(0.5).send().await?;

// Move with limited torque
ctx.motion().position(1.0).torque(0.3).send().await?;

// Combine speed and torque
ctx.motion().position(0.5).speed(1.0).torque(0.8).send().await?;
```

### Sensation

Sensation is a user-controlled value from -1.0 to 1.0 that patterns can use to vary their behavior. What it means is up to each pattern - it could control speed ratio, pause duration, depth, or anything else.

| Method                          | Description                                                    |
| ------------------------------- | -------------------------------------------------------------- |
| `ctx.sensation()`               | Returns the current sensation value (-1.0 to 1.0).             |
| `ctx.scale_sensation(min, max)` | Maps the sensation range (-1.0..1.0) linearly onto `min..max`. |

```rust
// Use sensation to control torque (0.0 at min sensation, 1.0 at max)
let torque = ctx.scale_sensation(0.0, 1.0);
ctx.motion().position(1.0).torque(torque).send().await?;

// Read raw sensation to branch on sign
let sensation = ctx.sensation();
let (out_speed, in_speed) = if sensation > 0.0 {
    (0.2, 1.0)
} else {
    (1.0, 0.2)
};
ctx.motion().position(1.0).speed(out_speed).send().await?;
ctx.motion().position(0.0).speed(in_speed).send().await?;

// Map sensation to a delay in milliseconds
let delay = ctx.scale_sensation(100.0, 10_000.0) as u64;
```

### Delay

| Method             | Description                                                 |
| ------------------ | ----------------------------------------------------------- |
| `ctx.delay_ms(ms)` | Pauses for `ms` milliseconds. **Not** a cancellation point. |

```rust
// Pause between bursts of strokes
for _ in 0..3 {
    ctx.motion().position(1.0).send().await?;
    ctx.motion().position(0.0).send().await?;
}
let delay = ctx.scale_sensation(100.0, 10_000.0) as u64;
ctx.delay_ms(delay).await;
```
