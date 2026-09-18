param(
  [string]$OutDir = "dist",
  [int]$RuntimeVersion = 3,
  [string]$Platform = "windows-x86_64"
)

$ErrorActionPreference = "Stop"
if (-not $IsWindows) { throw "WinApp runtime must be built on Windows" }
if ($env:PROCESSOR_ARCHITECTURE -notin @("AMD64", "IA64")) { throw "WinApp runtime build requires x64 Windows" }

$repo = Resolve-Path (Join-Path $PSScriptRoot "..")
$out = Join-Path $repo $OutDir
$stage = Join-Path $out "teshi-winapp-runtime-$Platform-v$RuntimeVersion"
$zip = Join-Path $out "teshi-winapp-runtime-$Platform-v$RuntimeVersion.zip"
Remove-Item -Recurse -Force $stage, $zip -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

# CI is expected to provide a controlled Python via actions/setup-python.
$python = (Get-Command python).Source
if (-not $python) { throw "python command not found in CI image" }

$pythonDir = Join-Path $stage "python"
New-Item -ItemType Directory -Force $pythonDir | Out-Null
$pythonSourceDir = Split-Path -Path $python -Parent
Copy-Item -Path (Join-Path $pythonSourceDir '*') -Destination $pythonDir -Recurse -Force

$runtimePython = Join-Path $pythonDir "python.exe"
& $runtimePython -m pip install --upgrade pip
& $runtimePython -m pip install -r (Join-Path $repo "python/winapp-requirements.txt")
& $runtimePython -m pip freeze | Set-Content -Encoding UTF8 (Join-Path $stage "requirements-lock.txt")

$resources = Join-Path $stage "resources"
New-Item -ItemType Directory -Force $resources | Out-Null
Copy-Item (Join-Path $repo "resources/winapp_service.py") (Join-Path $resources "winapp_service.py") -Force
Copy-Item (Join-Path $repo "resources/update_participant.py") (Join-Path $resources "update_participant.py") -Force

$manifest = [ordered]@{
  runtime = "winapp"
  runtime_version = $RuntimeVersion
  platform = $Platform
  python_exe = "python/python.exe"
  service_script = "resources/winapp_service.py"
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 (Join-Path $stage "runtime.json")

Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -Force
$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
$size = (Get-Item $zip).Length
[ordered]@{
  name = Split-Path $zip -Leaf
  runtime = "winapp"
  runtime_version = $RuntimeVersion
  platform = $Platform
  size = $size
  sha256 = $hash
} | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 "$zip.runtime-manifest.json"
Write-Host "WinApp runtime: $zip"
Write-Host "SHA256: $hash"
Write-Host "Size: $size"
