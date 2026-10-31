param(
    [Parameter(Mandatory)][ValidateSet('default', 'runner')][string]$Mode,
    [Parameter(Mandatory)][string]$Evidence
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Evidence | Out-Null
$before = @{}
foreach ($key in @('TEMP', 'TMP', 'RUNNER_TEMP')) {
    $before[$key] = [Environment]::GetEnvironmentVariable($key)
}
$target = $null
if ($Mode -eq 'runner') {
    if (-not $env:RUNNER_TEMP) { throw 'RUNNER_TEMP is required for runner mode' }
    $target = Join-Path $env:RUNNER_TEMP ('teshi-test-temp-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $target | Out-Null
    $env:TEMP = $target
    $env:TMP = $target
    if ($env:GITHUB_ENV) {
        "TEMP=$target" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
        "TMP=$target" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
    }
}

$paths = @($before.Values) + @($target, $env:GITHUB_WORKSPACE)
$disks = @($paths | Where-Object { $_ } | ForEach-Object {
    [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($_))
} | Select-Object -Unique | ForEach-Object {
    $root = $_
    $disk = [IO.DriveInfo]::new($root)
    @{
        root = $root
        drive_type = $disk.DriveType.ToString()
        format = $disk.DriveFormat
        available_bytes = $disk.AvailableFreeSpace
        total_bytes = $disk.TotalSize
    }
})

# Fresh child processes resolve temp paths after overrides; Python caches gettempdir.
$pythonTemp = & python -c 'import tempfile; print(tempfile.gettempdir())'
if ($LASTEXITCODE -ne 0) { throw 'Python temp probe failed' }
$probeSource = Join-Path $Evidence 'temp_probe.rs'
$probeBinary = Join-Path $Evidence 'temp-probe.exe'
'fn main() { println!("{}", std::env::temp_dir().display()); }' | Set-Content -LiteralPath $probeSource -Encoding utf8
& rustc --crate-name teshi_ci_temp_probe $probeSource -o $probeBinary
if ($LASTEXITCODE -ne 0) { throw 'Rust temp probe build failed' }
$rustTemp = & $probeBinary
if ($LASTEXITCODE -ne 0) { throw 'Rust temp probe failed' }
if ($target) {
    foreach ($actual in @($pythonTemp, $rustTemp)) {
        if ([IO.Path]::GetFullPath($actual).TrimEnd('\') -ne [IO.Path]::GetFullPath($target).TrimEnd('\')) {
            throw "Child temp path did not use the experiment directory: $actual"
        }
    }
}
@{
    mode = $Mode
    source_sha = (& git rev-parse HEAD)
    before = $before
    after = @{ TEMP = $env:TEMP; TMP = $env:TMP }
    python_temp = $pythonTemp
    rust_temp = $rustTemp
    disks = $disks
    runner = @{ os = $env:RUNNER_OS; arch = $env:RUNNER_ARCH; image = $env:ImageOS; image_version = $env:ImageVersion }
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Evidence 'temp-environment.json') -Encoding utf8
Get-Content -LiteralPath (Join-Path $Evidence 'temp-environment.json')
