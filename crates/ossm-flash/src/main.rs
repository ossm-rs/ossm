use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "ossm-flash", about = "Build, flash, and monitor OSSM firmware")]
struct Cli {
    /// Firmware variant to build and flash.
    variant: Variant,

    /// Motor backend to compile in. Defaults to the variant's hardware motor.
    /// Use `sim` to flash a build with the simulated motor instead.
    #[arg(long, value_enum)]
    motor: Option<Motor>,

    /// Skip the build step and flash whatever ELF is already on disk.
    #[arg(long, conflicts_with = "build_only")]
    no_build: bool,

    /// Build only; do not flash or monitor. Used by CI.
    #[arg(long)]
    build_only: bool,

    /// Override the serial port (otherwise espflash auto-detects).
    #[arg(long)]
    port: Option<String>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum Variant {
    OssmAlt,
    Waveshare,
    SeeedXiao,
    OssmReference,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
enum Motor {
    Rs485,
    Stepdir,
    Sim,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Indicator {
    None,
    Ws2812b,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Feature {
    Motor(Motor),
    Indicator(Indicator),
}

struct VariantSpec {
    workspace: &'static str,
    bin: &'static str,
    target: &'static str,
    motor: Motor,
    indicator: Indicator,
}

impl Variant {
    fn spec(self) -> VariantSpec {
        match self {
            Variant::OssmAlt => VariantSpec {
                workspace: "firmware/esp32s3",
                bin: "ossm-alt",
                target: "xtensa-esp32s3-none-elf",
                motor: Motor::Rs485,
                indicator: Indicator::Ws2812b,
            },
            Variant::Waveshare => VariantSpec {
                workspace: "firmware/esp32s3",
                bin: "waveshare",
                target: "xtensa-esp32s3-none-elf",
                motor: Motor::Rs485,
                indicator: Indicator::None,
            },
            Variant::SeeedXiao => VariantSpec {
                workspace: "firmware/esp32s3",
                bin: "seeed-xiao",
                target: "xtensa-esp32s3-none-elf",
                motor: Motor::Rs485,
                indicator: Indicator::None,
            },
            Variant::OssmReference => VariantSpec {
                workspace: "firmware/esp32",
                bin: "ossm-reference",
                target: "xtensa-esp32-none-elf",
                motor: Motor::Stepdir,
                indicator: Indicator::None,
            },
        }
    }
}

impl Motor {
    fn feature(self) -> &'static str {
        match self {
            Motor::Rs485 => "motor-rs485",
            Motor::Stepdir => "motor-stepdir",
            Motor::Sim => "motor-sim",
        }
    }
}

impl Indicator {
    fn feature(self) -> Option<&'static str> {
        match self {
            Indicator::None => None,
            Indicator::Ws2812b => Some("indicator-ws2812b"),
        }
    }
}

impl Feature {
    fn cargo_name(self) -> Option<&'static str> {
        match self {
            Feature::Motor(motor) => Some(motor.feature()),
            Feature::Indicator(indicator) => indicator.feature(),
        }
    }
}

impl VariantSpec {
    fn features(&self) -> Vec<Feature> {
        vec![
            Feature::Motor(self.motor),
            Feature::Indicator(self.indicator),
        ]
    }

    fn cargo_features(&self) -> String {
        self.features()
            .into_iter()
            .filter_map(Feature::cargo_name)
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn workspace_root() -> Result<PathBuf> {
    // ossm-flash lives at <root>/crates/ossm-flash; CARGO_MANIFEST_DIR points there.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .map(PathBuf::from)
        .context("could not resolve workspace root from CARGO_MANIFEST_DIR")
}

fn run_build(spec: &VariantSpec) -> Result<PathBuf> {
    let root = workspace_root()?;
    let workspace_dir = root.join(spec.workspace);
    let features = spec.cargo_features();

    eprintln!(
        "ossm-flash: building {} ({}) in {}",
        spec.bin,
        features,
        workspace_dir.display()
    );

    let status = Command::new("cargo")
        .current_dir(&workspace_dir)
        .args([
            "+esp",
            "build",
            "--release",
            "--bin",
            spec.bin,
            "--features",
            &features,
        ])
        .status()
        .context("failed to invoke `cargo +esp build` (is the esp toolchain installed?)")?;

    if !status.success() {
        bail!("cargo build failed for {}", spec.bin);
    }

    let elf = workspace_dir
        .join("target")
        .join(spec.target)
        .join("release")
        .join(spec.bin);

    if !elf.exists() {
        bail!("expected ELF not found at {}", elf.display());
    }

    Ok(elf)
}

fn run_flash_and_monitor(elf: &PathBuf, port: Option<&str>) -> Result<()> {
    eprintln!("ossm-flash: flashing {}", elf.display());

    let mut cmd = Command::new("espflash");
    cmd.arg("flash").arg("--monitor");
    if let Some(p) = port {
        cmd.arg("--port").arg(p);
    }
    cmd.arg(elf);

    // Inherit stdio so the user sees espflash's progress bar and the chip's
    // boot/log output live. espflash holds the serial FD across flash -> monitor
    // internally, so we don't lose ROM-bootloader bytes.
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .context("failed to invoke `espflash` (is it on PATH? `cargo install espflash`)")?;

    if !status.success() {
        bail!("espflash exited with status {status}");
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut spec = cli.variant.spec();
    if let Some(motor) = cli.motor {
        spec.motor = motor;
    }

    let elf = if cli.no_build {
        let root = workspace_root()?;
        root.join(spec.workspace)
            .join("target")
            .join(spec.target)
            .join("release")
            .join(spec.bin)
    } else {
        run_build(&spec)?
    };

    if cli.build_only {
        return Ok(());
    }

    run_flash_and_monitor(&elf, cli.port.as_deref())
}
