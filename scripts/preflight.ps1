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

function Resolve-Python {
    foreach ($candidate in @('python', 'python3')) {
        $cmd = Get-Command $candidate -ErrorAction SilentlyContinue
        if ($cmd) {
            & $cmd.Source -c 'import sys' 2>$null
            if ($LASTEXITCODE -eq 0) {
                return @($cmd.Source)
            }
        }
    }
    $py = Get-Command py -ErrorAction SilentlyContinue
    if ($py) {
        return @($py.Source, '-3')
    }
    throw 'preflight: python not found on PATH'
}

[string[]]$pythonCmd = Resolve-Python

Invoke-Gate @('cargo', 'fmt', '--all', '--check')
Invoke-Gate @('cargo', 'fmt', '--manifest-path', 'crates/teshi-tui-runner/Cargo.toml', '--check')

# Fast contract and partition gates catch catalog/CI regressions in seconds.
Invoke-Gate ($pythonCmd + @('scripts/check-feature-e2e-catalog.py'))
Invoke-Gate ($pythonCmd + @('scripts/run-ci-suite.py', 'check'))
Invoke-Gate ($pythonCmd + @('scripts/test-hosted-ui-transport.py'))
Invoke-Gate ($pythonCmd + @('scripts/test_update_manifest.py'))
Invoke-Gate ($pythonCmd + @('scripts/test-release-workflow-contract.py'))
Invoke-Gate ($pythonCmd + @('scripts/test-nightly-workflow-contract.py'))
Invoke-Gate ($pythonCmd + @('scripts/test-browser-agent-package.py'))
Invoke-Gate ($pythonCmd + @('-m', 'unittest', 'discover', '-s', 'scripts/tests', '-p', 'test_ci_*.py'))

if (-not $NoTests) {
    Invoke-Gate @('cargo', 'test', '--workspace', '--exclude', 'teshi-web', '--locked')
    Invoke-Gate @('cargo', 'test', '--manifest-path', 'crates/teshi-tui-runner/Cargo.toml', '--locked')
}

# Run --all-features clippy after default-feature tests (matching CI) to avoid feature-flip rebuilds.
Invoke-Gate @('cargo', 'clippy', '--workspace', '--exclude', 'teshi-web', '--locked', '--all-targets', '--all-features', '--', '-D', 'warnings')
Invoke-Gate @('cargo', 'clippy', '--manifest-path', 'crates/teshi-tui-runner/Cargo.toml', '--locked', '--all-targets', '--', '-D', 'warnings')

Write-Host 'preflight: OK'
