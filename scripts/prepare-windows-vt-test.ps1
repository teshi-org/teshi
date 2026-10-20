$ErrorActionPreference = 'Stop'

# Select Cargo's exact test artifact instead of guessing its hash or timestamp.
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $messages = & cargo test -p teshi-tui --lib --locked --no-run --message-format=json
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to build the Windows VT input test fixture'
    }
    $artifact = $messages | ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'teshi_tui' -and $_.executable } |
        Select-Object -Last 1
    if (-not $artifact) {
        throw 'Cargo did not report the teshi-tui test executable'
    }
    $artifact.executable
}
finally {
    Pop-Location
}
