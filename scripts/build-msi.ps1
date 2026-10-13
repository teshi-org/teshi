# Build full teshi MSI (matches CI release layout).
# Usage: pwsh -File scripts/build-msi.ps1
param(
    [string] $BinaryDir = "target/release",
    [string] $WebDist = "apps/teshi-web/dist",
    [string] $StagingRoot = "staging/msi-root",
    [string] $OutputDir = "target/wix",
    [switch] $SkipBuild,
    [switch] $SkipWebBuild
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$cargoToml = Get-Content -Raw -Path "Cargo.toml"
if ($cargoToml -notmatch '(?ms)\[workspace\.package\].*?^version = "([^"]+)"') {
    throw "workspace package version is missing from Cargo.toml"
}
$version = $Matches[1]
$tag = "v$version"
Write-Host "==> Building teshi $tag MSI"

if (-not $env:WIX) {
    throw "WiX Toolset not found. Set WIX env or install WiX Toolset v3.14."
}
$heat = Join-Path $env:WIX "bin/heat.exe"
if (-not (Test-Path $heat)) {
    throw "Missing heat.exe at $heat"
}

if (-not $SkipWebBuild) {
    Write-Host "==> Frontend (GPUI WASM)"
    & (Join-Path $PSScriptRoot "build-teshi-web.ps1")
    if ($LASTEXITCODE -ne 0) { throw "GPUI WASM build failed: $LASTEXITCODE" }
}

if (-not $SkipBuild) {
    Write-Host "==> Native binaries (release)"
    cargo build --release --locked --bin teshi --bin teshi-update-helper --bin teshi-desktop
    if ($LASTEXITCODE -ne 0) { throw "Native release build failed: $LASTEXITCODE" }
}

$builtExe = Join-Path $BinaryDir "teshi.exe"
foreach ($p in @($builtExe, (Join-Path $BinaryDir "teshi-update-helper.exe"), (Join-Path $BinaryDir "teshi-desktop.exe"), (Join-Path $WebDist "index.html"), (Join-Path $WebDist "pkg/teshi_web_bg.wasm"), "resources/browser_service.py", "resources/browser_agent_broker.py", "resources/winapp_service.py", "resources/update_participant.py", "resources/api_service.py", "resources/teshi_api.py", "agent-packages/teshi-browser-testing/.codex-plugin/plugin.json")) {
    if (-not (Test-Path $p)) { throw "Missing $p" }
}

Write-Host "==> Stage MSI root"
# Refuse stale staging contents rather than merging unknown local files into
# the installer. Callers can select a new staging directory without deleting it.
if ((Test-Path -LiteralPath $StagingRoot) -and (Get-ChildItem -LiteralPath $StagingRoot -Force)) {
    throw "Staging root is not empty: $StagingRoot. Select a fresh -StagingRoot."
}
$stagingRoot = $StagingRoot
$binDir = Join-Path $stagingRoot "bin"
$shareDir = Join-Path $stagingRoot "share"
$webDir = Join-Path $stagingRoot "share/web"
$bridgeDir = Join-Path $stagingRoot "share/teshi-bridge"
$agentDir = Join-Path $stagingRoot "share/teshi-browser-testing"
$agentBridgeDir = Join-Path $agentDir "extension/teshi-bridge"
New-Item -ItemType Directory -Force -Path $binDir, $webDir, $bridgeDir, $agentBridgeDir | Out-Null

Copy-Item $builtExe (Join-Path $binDir "teshi.exe") -Force
Copy-Item (Join-Path $BinaryDir "teshi-update-helper.exe") (Join-Path $binDir "teshi-update-helper.exe") -Force
Copy-Item (Join-Path $BinaryDir "teshi-desktop.exe") (Join-Path $binDir "teshi-desktop.exe") -Force
python scripts/package_runtime.py --destination $shareDir --entry-point browser_service.py --entry-point api_service.py --entry-point winapp_service.py
if ($LASTEXITCODE -ne 0) { throw "Runtime resource staging failed" }
Copy-Item -Path (Join-Path $WebDist "*") -Destination $webDir -Recurse -Force
Copy-Item -Path "extension/teshi-bridge/*" -Destination $bridgeDir -Recurse -Force
Copy-Item -Path "agent-packages/teshi-browser-testing/*" -Destination $agentDir -Recurse -Force
Copy-Item -LiteralPath "skills/teshi", "skills/bdd-feature", "skills/winapp-regression" -Destination (Join-Path $agentDir "skills") -Recurse -Force
Copy-Item -Path "agent-packages/teshi-browser-testing/.codex-plugin" -Destination $agentDir -Recurse -Force
Copy-Item -Path "agent-packages/teshi-browser-testing/.mcp.json" -Destination $agentDir -Force
Copy-Item -Path "extension/teshi-bridge/*" -Destination $agentBridgeDir -Recurse -Force

# Use the compiled identity, also when packaging prebuilt binaries. Never emit
# a manifest that claims a different build than its shipped executables.
$identityText = & $builtExe --update-identity
if ($LASTEXITCODE -ne 0) { throw "Cannot read CLI build identity" }
$identity = $identityText | ConvertFrom-Json
if ($identity.semver -ne $version) { throw "CLI/workspace version mismatch" }
foreach ($binary in @("teshi-update-helper.exe", "teshi-desktop.exe")) {
    $otherText = & (Join-Path $BinaryDir $binary) --update-identity
    if ($LASTEXITCODE -ne 0) { throw "Cannot read build identity: $binary" }
    $other = $otherText | ConvertFrom-Json
    foreach ($field in @("semver", "channel", "git_sha", "build_timestamp", "build_sequence")) {
        if ($other.$field -ne $identity.$field) { throw "Build identity mismatch: $binary ($field)" }
    }
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$identityFile = Join-Path $OutputDir "build-identity.json"
$identityText | Set-Content -Encoding utf8 -Path $identityFile
python scripts/update_manifest.py bundle --root $stagingRoot --identity $identityFile --target x86_64-pc-windows-msvc --kind msi
if ($LASTEXITCODE -ne 0) { throw "MSI bundle manifest generation failed" }
python scripts/test-release-package.py --root $stagingRoot --target x86_64-pc-windows-msvc --kind msi --run-cli
if ($LASTEXITCODE -ne 0) { throw "MSI payload validation failed" }

Write-Host "==> heat web + bridge + browser-agent fragments"
$fragmentDir = Join-Path $OutputDir "fragments"
New-Item -ItemType Directory -Force -Path $fragmentDir | Out-Null
$webFragment = Join-Path $fragmentDir "web-files.wxs"
$bridgeFragment = Join-Path $fragmentDir "bridge-files.wxs"
$agentFragment = Join-Path $fragmentDir "browser-agent-files.wxs"
& $heat dir $webDir `
    -out $webFragment `
    -gg -sfrag -srd `
    -dr ShareWebDir `
    -cg WebFiles `
    -var var.WebRoot
if ($LASTEXITCODE -ne 0) { throw "Web harvest failed" }
& $heat dir $bridgeDir `
    -out $bridgeFragment `
    -gg -sfrag -srd `
    -dr ShareTeshiBridgeDir `
    -cg BridgeFiles `
    -var var.BridgeRoot
if ($LASTEXITCODE -ne 0) { throw "Bridge harvest failed" }
& $heat dir $agentDir `
    -out $agentFragment `
    -gg -sfrag -srd `
    -dr ShareTeshiAgentDir `
    -cg BrowserAgentFiles `
    -var var.AgentRoot
if ($LASTEXITCODE -ne 0) { throw "Browser-agent harvest failed" }

Write-Host "==> cargo wix"
$stagingFull = (Resolve-Path -LiteralPath $stagingRoot).Path
$webFull = (Resolve-Path -LiteralPath $webDir).Path
$bridgeFull = (Resolve-Path -LiteralPath $bridgeDir).Path
$agentFull = (Resolve-Path -LiteralPath $agentDir).Path
$outputFile = Join-Path $OutputDir "teshi-$tag-x64.msi"
cargo wix --package teshi-cli `
    --include wix/main.wxs `
    --include $webFragment `
    --include $bridgeFragment `
    --include $agentFragment `
    "--compiler-arg=-dStagingRoot=$stagingFull" `
    "--compiler-arg=-dWebRoot=$webFull" `
    "--compiler-arg=-dBridgeRoot=$bridgeFull" `
    "--compiler-arg=-dAgentRoot=$agentFull" `
    --nocapture --no-build --target-bin-dir $BinaryDir -o $outputFile
if ($LASTEXITCODE -ne 0) { throw "cargo wix failed: $LASTEXITCODE" }

Write-Host "==> Done: $outputFile"
