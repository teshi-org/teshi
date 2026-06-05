# Build full teshi MSI (matches CI release layout).
# Usage: pwsh -File scripts/build-msi.ps1
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
$tag = "v$version"
Write-Host "==> Building teshi $tag MSI"

if (-not $env:WIX) {
    throw "WiX Toolset not found. Set WIX env or install WiX Toolset v3.14."
}
$heat = Join-Path $env:WIX "bin/heat.exe"
if (-not (Test-Path $heat)) {
    throw "Missing heat.exe at $heat"
}

Write-Host "==> Frontend"
Push-Location desktop
npm ci
npm run build
Pop-Location

Write-Host "==> teshi CLI (release)"
cargo build --release --bin teshi

Write-Host "==> teshi-desktop (release, no bundle)"
Push-Location desktop
npx tauri build --no-bundle
Pop-Location

$builtExe = "target/release/teshi.exe"
$desktopExe = "target/release/teshi-desktop.exe"
foreach ($p in @($builtExe, $desktopExe, "desktop/dist/index.html")) {
    if (-not (Test-Path $p)) { throw "Missing $p" }
}

Write-Host "==> Stage MSI root"
$stagingRoot = "staging/msi-root"
$binDir = Join-Path $stagingRoot "bin"
$shareDir = Join-Path $stagingRoot "share"
$webDir = Join-Path $stagingRoot "share/web"
$bridgeDir = Join-Path $stagingRoot "share/teshi-bridge"
New-Item -ItemType Directory -Force -Path $binDir, $webDir, $bridgeDir | Out-Null

Copy-Item $builtExe (Join-Path $binDir "teshi.exe") -Force
Copy-Item $desktopExe (Join-Path $binDir "teshi-desktop.exe") -Force
Copy-Item "desktop/src-tauri/resources/browser_service.py" (Join-Path $shareDir "browser_service.py") -Force
Copy-Item "desktop/src-tauri/resources/winapp_service.py" (Join-Path $shareDir "winapp_service.py") -Force
Copy-Item -Path "desktop/dist/*" -Destination $webDir -Recurse -Force
Copy-Item -Path "extension/teshi-bridge/*" -Destination $bridgeDir -Recurse -Force

Write-Host "==> heat web + bridge fragments"
& $heat dir $webDir `
    -out wix/web-files.wxs `
    -gg -sfrag -srd `
    -dr ShareWebDir `
    -cg WebFiles `
    -var var.WebRoot
& $heat dir $bridgeDir `
    -out wix/bridge-files.wxs `
    -gg -sfrag -srd `
    -dr ShareTeshiBridgeDir `
    -cg BridgeFiles `
    -var var.BridgeRoot

$defaultBlock = @"

<?ifndef WebRoot ?>
	<?define WebRoot = "staging\msi-root\share\web" ?>
<?endif ?>
<?ifndef BridgeRoot ?>
	<?define BridgeRoot = "staging\msi-root\share\teshi-bridge" ?>
<?endif ?>
"@
foreach ($f in @("wix/web-files.wxs", "wix/bridge-files.wxs")) {
    $content = Get-Content $f -Raw
    $idx = $content.IndexOf("`n")
    if ($idx -ge 0) {
        $content = $content.Substring(0, $idx) + $defaultBlock + $content.Substring($idx)
    }
    Set-Content $f -Value $content -NoNewline
}

$outDir = "target/wix"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$msiPath = Join-Path $outDir "teshi-$tag-x64.msi"

Write-Host "==> cargo wix -> $msiPath"
cargo wix --package teshi --nocapture --no-build `
    -C "-dStagingRoot=staging/msi-root" `
    -C "-dWebRoot=staging/msi-root/share/web" `
    -C "-dBridgeRoot=staging/msi-root/share/teshi-bridge" `
    -o $msiPath

Write-Host "MSI ready: $msiPath"
