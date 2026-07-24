# Build GPUI WASM shell into apps/teshi-web/dist for Path 1 (daemon --dist).
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$OutDir = Join-Path $Root "apps\teshi-web\dist"
$PkgDir = Join-Path $OutDir "pkg"

Write-Host "==> building teshi-web (nightly, wasm32-unknown-unknown)"
rustup target add wasm32-unknown-unknown --toolchain nightly | Out-Null
cargo +nightly build --release --target wasm32-unknown-unknown -p teshi-web
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Wasm = Join-Path $TargetDir "wasm32-unknown-unknown\release\teshi_web.wasm"
if (-not (Test-Path $Wasm)) {
    throw "missing $Wasm"
}

Write-Host "==> wasm-bindgen → $PkgDir"
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
New-Item -ItemType Directory -Path $PkgDir | Out-Null
wasm-bindgen $Wasm --target web --out-dir $PkgDir
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Copy-Item (Join-Path $Root "apps\teshi-web\web\index.html") (Join-Path $OutDir "index.html")
Copy-Item (Join-Path $Root "apps\teshi-web\web\main.js") (Join-Path $OutDir "main.js")

Write-Host "==> done: $OutDir"
Write-Host "Path 1: cargo run -p teshi-cli -- web --no-open --dist `"$OutDir`""
