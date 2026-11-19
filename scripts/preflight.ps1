#!/usr/bin/env pwsh
# Run the same quality gates CI enforces, before pushing.
#
# The terminal runner is a separate Cargo workspace, so it has its own
# formatting and lint commands. teshi-web is wasm32-only and must stay out of
# native workspace commands.
[CmdletBinding()]
param([switch]$NoTests)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Invoke-Gate {
    param([Parameter(Mandatory)][string[]]$Command)
    Write-Host ":: $($Command -join ' ')"
    & $Command[0] @($Command[1..($Command.Length - 1)])
    if ($LASTEXITCODE -ne 0) {
        throw "gate failed: $($Command -join ' ')"
    }
}

Invoke-Gate @('cargo', 'fmt', '--all', '--check')
Invoke-Gate @('cargo', 'fmt', '--manifest-path', 'crates/teshi-tui-runner/Cargo.toml', '--check')
Invoke-Gate @('cargo', 'clippy', '--workspace', '--exclude', 'teshi-web', '--locked', '--all-targets', '--all-features', '--', '-D', 'warnings')
Invoke-Gate @('cargo', 'clippy', '--manifest-path', 'crates/teshi-tui-runner/Cargo.toml', '--locked', '--all-targets', '--', '-D', 'warnings')

if (-not $NoTests) {
    Invoke-Gate @('cargo', 'test', '--workspace', '--exclude', 'teshi-web', '--locked')
    Invoke-Gate @('cargo', 'test', '--manifest-path', 'crates/teshi-tui-runner/Cargo.toml', '--locked')
}

Write-Host 'preflight: OK'
