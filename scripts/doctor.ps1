$ok = $true

function Get-ToolVersion($cmd) {
    try { @(& $cmd --version 2>$null)[0] } catch { "" }
}

function Check($name, $reason) {
    $found = Get-Command $name -ErrorAction SilentlyContinue
    if ($found) {
        $ver = Get-ToolVersion $name
        Write-Host "  + " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-12} {1}" -f $name, $ver)
    } else {
        Write-Host "  x " -ForegroundColor Red -NoNewline
        Write-Host ("{0,-12} {1}" -f $name, $reason)
        $script:ok = $false
    }
}

function CheckOptional($name, $reason) {
    $found = Get-Command $name -ErrorAction SilentlyContinue
    if ($found) {
        $ver = Get-ToolVersion $name
        Write-Host "  + " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-12} {1}" -f $name, $ver)
    } else {
        Write-Host "  ~ " -ForegroundColor Yellow -NoNewline
        Write-Host ("{0,-12} {1} (optional)" -f $name, $reason)
    }
}

function CheckEspToolchain() {
    $espVer = @(& cargo +esp --version 2>&1)[0]
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  + " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-12} {1}" -f "+esp", $espVer)
    } else {
        Write-Host "  x " -ForegroundColor Red -NoNewline
        Write-Host ("{0,-12} {1}" -f "+esp", "needed to cross-compile for ESP32 targets")
        $script:ok = $false
    }
}

function CheckExportEsp() {
    $exportPath = Join-Path $env:USERPROFILE "export-esp.ps1"
    if (Test-Path $exportPath) {
        Write-Host "  + " -ForegroundColor Green -NoNewline
        Write-Host "~/export-esp.ps1"
    } else {
        Write-Host "  x " -ForegroundColor Red -NoNewline
        Write-Host "~/export-esp.ps1 not found - needed to set ESP toolchain paths"
        $script:ok = $false
    }
}

function CheckWasmTarget() {
    $targets = @(& rustup +stable target list --installed 2>$null)
    if ($targets -contains "wasm32-unknown-unknown") {
        Write-Host "  + " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-12} {1}" -f "wasm32", "wasm32-unknown-unknown target installed")
    } else {
        Write-Host "  x " -ForegroundColor Red -NoNewline
        Write-Host ("{0,-12} {1}" -f "wasm32", "missing wasm32-unknown-unknown target (rustup +stable target add wasm32-unknown-unknown)")
        $script:ok = $false
    }
}

# Mirrors scripts/wasm-bindgen-version.sh: the wasm-bindgen crate version each
# binding's Cargo.lock pins.
function Get-LockedWasmBindgen() {
    $bindings = Join-Path (Split-Path -Parent $PSScriptRoot) "bindings"
    foreach ($dir in Get-ChildItem $bindings -Directory) {
        $lock = Join-Path $dir.FullName "Cargo.lock"
        if (-not (Test-Path $lock)) { continue }
        $match = Select-String -Path $lock -Pattern '^name = "wasm-bindgen"$' -Context 0,1 | Select-Object -First 1
        if ($match) {
            [pscustomobject]@{
                Lock    = "bindings/$($dir.Name)/Cargo.lock"
                Version = $match.Context.PostContext[0] -replace '^version = "(.*)"$', '$1'
            }
        }
    }
}

# The wasm-bindgen CLI refuses to run unless it exactly matches the crate
# version in the bindings' Cargo.lock, so check the version, not just presence.
function CheckWasmBindgen() {
    $locked = @(Get-LockedWasmBindgen)
    $versions = @($locked | ForEach-Object { $_.Version } | Sort-Object -Unique)
    if ($versions.Count -ne 1) {
        Write-Host "  x " -ForegroundColor Red -NoNewline
        Write-Host ("{0,-12} {1}" -f "wasm-bindgen", "can't tell which CLI version the bindings need:")
        if ($versions.Count -eq 0) {
            Write-Host "      no Cargo.lock under bindings/ depends on wasm-bindgen"
        } else {
            Write-Host "      bindings lock different wasm-bindgen versions:"
            foreach ($l in $locked) { Write-Host ("        {0,-40} {1}" -f $l.Lock, $l.Version) }
        }
        $script:ok = $false
        return
    }
    $expected = $versions[0]

    if (-not (Get-Command wasm-bindgen -ErrorAction SilentlyContinue)) {
        Write-Host "  x " -ForegroundColor Red -NoNewline
        Write-Host ("{0,-12} {1}" -f "wasm-bindgen", "needed to build the WASM simulator (cargo install wasm-bindgen-cli --locked --version $expected)")
        $script:ok = $false
        return
    }

    $installed = ""
    if ((Get-ToolVersion wasm-bindgen) -match '\d+\.\d+\.\d+') { $installed = $Matches[0] }
    if ($installed -eq $expected) {
        Write-Host "  + " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-12} {1}" -f "wasm-bindgen", "$installed (matches Cargo.lock)")
        return
    }

    if (-not $installed) { $installed = "unknown version" }
    $found = (Get-Command wasm-bindgen).Source
    $foundDir = Split-Path -Parent $found
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
    $cargoBin = Join-Path $cargoHome "bin"
    $install = "cargo install wasm-bindgen-cli --locked --version $expected --force"
    Write-Host "  x " -ForegroundColor Red -NoNewline
    Write-Host ("{0,-12} {1}" -f "wasm-bindgen", "$installed found, bindings need $expected")
    Write-Host "      found at $found"
    # Reinstalling with cargo only helps if cargo's copy is the one on PATH.
    if ($foundDir -eq $cargoBin) {
        Write-Host "      run: $install"
    } else {
        Write-Host "      this one was installed by something other than cargo (e.g. a package"
        Write-Host "      manager); update it there, or run $install"
        Write-Host "      and put $cargoBin ahead of $foundDir on PATH"
    }
    $script:ok = $false
}

Write-Host "Firmware..."
Check cargo            "needed to compile Rust crates"
Check espup            "needed to install the ESP Rust toolchain"
Check espflash         "needed to flash firmware to ESP32 boards"
CheckEspToolchain
CheckExportEsp

Write-Host ""
Write-Host "Web simulator..."
Check node             "needed as the JS runtime for pnpm"
Check pnpm             "needed to run the web simulator dev server"
CheckWasmBindgen
Check wasm-opt         "needed to optimise WASM output (install binaryen)"
CheckWasmTarget

Write-Host ""
if ($ok) {
    Write-Host "All good!" -ForegroundColor Green
} else {
    Write-Host "Some checks failed. See above for details." -ForegroundColor Red
    exit 1
}
