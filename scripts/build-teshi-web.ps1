# Build GPUI WASM shell into apps/teshi-web/dist for Path 1 (daemon --dist).
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$OutDir = Join-Path $Root "apps\teshi-web\dist"
$PkgDir = Join-Path $OutDir "pkg"
$CacheBust = Get-Date -Format "yyyyMMddHHmmss"

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

# Browser caches WASM aggressively; stamp asset URLs so rebuilds take effect.
$BindgenJs = Join-Path $PkgDir "teshi_web.js"
$BindgenText = Get-Content -Raw $BindgenJs
$BindgenText = $BindgenText.Replace(
    "new URL('teshi_web_bg.wasm', import.meta.url)",
    "new URL('teshi_web_bg.wasm?v=$CacheBust', import.meta.url)"
)
Set-Content -Path $BindgenJs -Value $BindgenText -NoNewline

$MainJs = @"
import init, { run } from "./pkg/teshi_web.js?v=$CacheBust";

const loading = document.getElementById("loading");

try {
  await init();
  run();
  loading?.setAttribute("hidden", "");
} catch (err) {
  console.error(err);
  if (loading) {
    loading.classList.add("error");
    loading.textContent = "Failed to start teshi-web: " + err;
  }
}
"@
Set-Content -Path (Join-Path $OutDir "main.js") -Value $MainJs

$IndexHtml = Get-Content -Raw (Join-Path $Root "apps\teshi-web\web\index.html")
$IndexHtml = $IndexHtml.Replace(
    'src="./main.js"',
    "src=`"./main.js?v=$CacheBust`""
)
Set-Content -Path (Join-Path $OutDir "index.html") -Value $IndexHtml -NoNewline

Write-Host "==> done: $OutDir (cache bust $CacheBust)"
Write-Host "Path 1: cargo run -p teshi-cli -- web --no-open --dist `"$OutDir`""
Write-Host "Open: http://127.0.0.1:20253/index.html?v=$CacheBust  (or hard-refresh if an old tab is open)"
