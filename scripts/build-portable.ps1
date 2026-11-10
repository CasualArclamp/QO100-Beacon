# made by CasualArclamp
# Builds a single-file qo100-beacon.exe for 64-bit Windows that needs nothing
# installed (the C runtime is linked statically), into exe\ (git-ignored).
#
#   powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$triple = "x86_64-pc-windows-msvc"

# Its own target directory, so the usual one and its cache stay as they are.
$env:CARGO_TARGET_DIR = Join-Path $root "target\portable"
$env:RUSTFLAGS = "-C target-feature=+crt-static"

Push-Location $root
try {
    cargo build --release --target $triple
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    Pop-Location
}

$out = Join-Path $root "exe"
New-Item -ItemType Directory -Force $out | Out-Null
$exe = Join-Path $env:CARGO_TARGET_DIR "$triple\release\qo100-beacon.exe"
Copy-Item $exe $out -Force
$size = [math]::Round((Get-Item (Join-Path $out "qo100-beacon.exe")).Length / 1MB, 1)
Write-Host "qo100-beacon.exe    $size MB"
Write-Host "written to $out"
