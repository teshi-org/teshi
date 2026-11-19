$ErrorActionPreference = 'Stop'
$demo = $PSScriptRoot
$repo = (Resolve-Path (Join-Path $demo '..\..')).Path
$binary = Join-Path $repo 'target\debug\teshi.exe'

if (-not (Test-Path $binary)) {
    Push-Location $repo
    try {
        cargo build -p teshi-cli
    } finally {
        Pop-Location
    }
}

Push-Location $demo
try {
    & $binary .
} finally {
    Pop-Location
}
