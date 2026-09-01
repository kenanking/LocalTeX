param(
    [string]$Version = "",
    [string]$OutDir = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $Root

if (-not $Version) {
    $line = Select-String -Path (Join-Path $Root "Cargo.toml") -Pattern '^version = "([^"]+)"' |
        Select-Object -First 1
    if (-not $line) { throw "localtex: no version in Cargo.toml" }
    $Version = $line.Matches[0].Groups[1].Value
}

$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$Arch = switch ($osArch) {
    "X64" { "x86_64" }
    "Arm64" { "aarch64" }
    default { throw "localtex: unsupported Windows arch $osArch" }
}
$Target = "$Arch-pc-windows-msvc"

if (-not $OutDir) { $OutDir = Join-Path $Root "target\release" }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

if ($env:LOCALTEX_SKIP_BUILD -eq "1") { $SkipBuild = $true }
$Bin = if ($env:LOCALTEX_BUNDLE_BIN) {
    $env:LOCALTEX_BUNDLE_BIN
} else {
    Join-Path $Root "target\release\localtex.exe"
}

if (-not $SkipBuild) {
    cargo build --locked --release --bin localtex
    if ($LASTEXITCODE -ne 0) { throw "localtex: cargo build failed" }
}
if (-not (Test-Path $Bin)) { throw "localtex: missing $Bin" }

$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ("localtex-win-" + [guid]::NewGuid().ToString("N"))
$Payload = Join-Path $Stage "payload"
New-Item -ItemType Directory -Force -Path $Payload | Out-Null
try {
    Copy-Item $Bin (Join-Path $Payload "localtex.exe")
    Copy-Item (Join-Path $Root "LICENSE") (Join-Path $Payload "LICENSE")
    $Models = Join-Path $Payload "models"
    $Bash = Get-Command bash -ErrorAction SilentlyContinue
    if (-not $Bash) {
        throw "localtex: bash is required to stage models (Git for Windows)"
    }
    & bash (Join-Path $Root "scripts\stage-models.sh") $Models
    if ($LASTEXITCODE -ne 0) { throw "localtex: stage-models.sh failed" }
    if (-not (Test-Path (Join-Path $Models "opendoc\layout.onnx"))) {
        throw "localtex: staged models missing opendoc/layout.onnx"
    }

    $ZipName = "localtex-$Version-$Target.zip"
    $ZipPath = Join-Path $OutDir $ZipName
    if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
    $ZipRoot = Join-Path $Stage "localtex-$Version-$Target"
    New-Item -ItemType Directory -Force -Path $ZipRoot | Out-Null
    Copy-Item -Path (Join-Path $Payload "*") -Destination $ZipRoot -Recurse
    Compress-Archive -Path $ZipRoot -DestinationPath $ZipPath
    Write-Host "localtex: wrote $ZipPath"

    $Iss = Join-Path $Root "resources\windows\localtex.iss"
    $Iscc = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Iscc) {
        $found = Get-Command iscc -ErrorAction SilentlyContinue
        if ($found) { $Iscc = $found.Source }
    }
    if (-not $Iscc) { throw "localtex: Inno Setup 6 ISCC.exe not found" }

    & $Iscc $Iss `
        "/DMyAppVersion=$Version" `
        "/DMyAppOutputDir=$OutDir" `
        "/DMyAppSourceDir=$Payload" `
        "/DMyAppArch=$Arch"
    if ($LASTEXITCODE -ne 0) { throw "localtex: ISCC failed with $LASTEXITCODE" }
    Write-Host "localtex: wrote $(Join-Path $OutDir "LocalTeX-$Version-$Arch-Setup.exe")"
} finally {
    if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
}
