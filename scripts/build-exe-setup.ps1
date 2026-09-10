# Builds the per-user Windows setup.exe from a staged EXE bundle tree.
param(
    [Parameter(Mandatory = $true)]
    [string] $StagingRoot,
    [Parameter(Mandatory = $true)]
    [string] $Tag,
    [Parameter(Mandatory = $true)]
    [string] $Version,
    [string] $OutputDir = "target/inno",
    [switch] $CliOnly
)

$ErrorActionPreference = "Stop"
$iscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    $iscc = Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe"
}
if (-not (Test-Path $iscc)) {
    throw "Inno Setup 6 ISCC.exe not found. Install with: choco install innosetup"
}

$repo = Split-Path -Parent $PSScriptRoot
$iss = Join-Path $repo "inno\teshi.iss"
$staging = (Resolve-Path $StagingRoot).Path
$out = Join-Path $repo $OutputDir
New-Item -ItemType Directory -Force -Path $out | Out-Null
$outputName = "teshi-$Tag-x64-setup"

& $iscc `
    "/DAppVersion=$Version" `
    "/DCliOnly=$(if ($CliOnly) { 1 } else { 0 })" `
    "/DSourceRoot=$staging" `
    "/DOutputDir=$out" `
    "/DOutputName=$outputName" `
    $iss
if ($LASTEXITCODE -ne 0) {
    throw "ISCC failed with exit code $LASTEXITCODE"
}

$built = Join-Path $out "$outputName.exe"
if (-not (Test-Path $built)) {
    throw "Missing setup payload: $built"
}
Write-Host "Built $built"
