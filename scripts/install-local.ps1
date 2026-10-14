# Installs the teshi CLI from this checkout onto PATH so every shell and agent
# tool resolves the same binary.
#
# `--profile dev` is deliberate. `cargo install` defaults to `release`, which
# enables `lto = true` (Cargo.toml). Every iteration then pays a full LTO link
# (~4 min) and cannot reuse the `target/debug` cache that `cargo build` fills.
# With the dev profile the install stays incremental: ~14 s after a one-line
# edit, ~4 s when nothing changed.
#
# Build the packaged installer (scripts/build-exe-setup.ps1) only when the
# installer or the update path itself is what you are validating.
param(
    # Reinstall even when cargo considers the package already installed.
    [switch] $Force
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

# Let Cargo detect whether its actual installation target is locked. Never
# terminate running editors: they may contain unsaved changes or belong to
# another installation. A locked target must be closed normally before retrying.

$cargoArgs = @('install', '--path', 'apps/teshi-cli', '--profile', 'dev')
if ($Force) { $cargoArgs += '--force' }
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
    throw "cargo install failed with exit code $LASTEXITCODE. If Cargo reports a locked executable or access denied, save your work and close the Teshi instance at Cargo's installation target, then retry."
}

$installed = Join-Path ${env:USERPROFILE} '.cargo\bin\teshi.exe'
$resolved = (Get-Command teshi -ErrorAction SilentlyContinue).Source
if ($resolved -and ($resolved -ne $installed)) {
    Write-Warning "PATH resolves 'teshi' to '$resolved' instead of the build installed here ('$installed')."
    Write-Warning "Reorder PATH or uninstall the other copy so agents pick up this build."
}

Write-Host "Installed: $installed"
& $installed --version
