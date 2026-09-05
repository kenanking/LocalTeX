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
    default { throw "localtex: unsupported Windows arch $osArch" }
}
$Target = "$Arch-pc-windows-msvc"

if (-not $OutDir) { $OutDir = Join-Path $Root "dist" }
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
$pe = [IO.File]::OpenRead($Bin)
try {
    $reader = [IO.BinaryReader]::new($pe)
    $pe.Position = 0x3c
    $header = $reader.ReadInt32()
    $pe.Position = $header
    if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
        throw "localtex: bundle requires an x64 PE executable"
    }
} finally {
    $pe.Dispose()
}

$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ("localtex-win-" + [guid]::NewGuid().ToString("N"))
$Payload = Join-Path $Stage "payload"
New-Item -ItemType Directory -Force -Path $Payload | Out-Null
try {
    Copy-Item $Bin (Join-Path $Payload "localtex.exe")
    Copy-Item (Join-Path $Root "LICENSE") (Join-Path $Payload "LICENSE")
    $Models = Join-Path $Payload "models"
    & (Join-Path $Root "scripts\download-models.ps1") $Models
    if ($LASTEXITCODE -ne 0) { throw "localtex: download-models.ps1 failed" }
    if (-not (Test-Path (Join-Path $Models "opendoc\layout.onnx"))) {
        throw "localtex: models missing opendoc/layout.onnx"
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
        "${env:ProgramFiles}\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
        "${env:LOCALAPPDATA}\Programs\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:LOCALAPPDATA}\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Iscc) {
        $found = Get-Command iscc -ErrorAction SilentlyContinue
        if ($found) { $Iscc = $found.Source }
    }
    if (-not $Iscc) { throw "localtex: Inno Setup ISCC.exe not found (install Inno Setup 7)" }
    Write-Host "localtex: using $Iscc"

    $SetupIcon = Join-Path $Root "target\localtex.ico"
    if (-not (Test-Path -LiteralPath $SetupIcon -PathType Leaf)) {
        $foundIco = Get-ChildItem -Path (Join-Path $Root "target") -Recurse -Filter "localtex.ico" -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if ($foundIco) { $SetupIcon = $foundIco.FullName }
    }
    if (-not (Test-Path -LiteralPath $SetupIcon -PathType Leaf)) {
        throw "localtex: missing target\localtex.ico (run a Windows cargo build once)"
    }
    $SetupIconDefine = $SetupIcon.Replace("\", "/")
    Write-Host "localtex: setup icon $SetupIcon"

    & $Iscc $Iss `
        "/DMyAppVersion=$Version" `
        "/DMyAppOutputDir=$OutDir" `
        "/DMyAppSourceDir=$Payload" `
        "/DMyAppArch=$Arch" `
        "/DMyAppSetupIcon=$SetupIconDefine"
    if ($LASTEXITCODE -ne 0) { throw "localtex: ISCC failed with $LASTEXITCODE" }
    Write-Host "localtex: wrote $(Join-Path $OutDir "LocalTeX-$Version-$Arch-Setup.exe")"
} finally {
    if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
}
