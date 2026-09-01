# Fetch OpenDoc + handwriting packs, or copy a local cache into DEST.
# Override with LOCALTEX_MODELS, LOCALTEX_MODELS_REPO, LOCALTEX_MODELS_TAG.
param(
    [Parameter(Position = 0)]
    [string]$Dest = ""
)

$ErrorActionPreference = "Stop"

$Repo = if ($env:LOCALTEX_MODELS_REPO) { $env:LOCALTEX_MODELS_REPO } else { "kenanking/LocalTeX" }
$Tag = if ($env:LOCALTEX_MODELS_TAG) { $env:LOCALTEX_MODELS_TAG } else { "v0.0.0" }
$OpenDocDir = "opendoc_int8_20260827_45af38b"
$HandwritingDir = "handwriting_e10_20260831_cf27b99"
$OpenDocTar = "$OpenDocDir.tar.gz"
$HandwritingTar = "$HandwritingDir.tar.gz"
$Sums = "SHA256SUMS"

function Get-DefaultModelsDir {
    if ($env:LOCALAPPDATA) {
        return (Join-Path $env:LOCALAPPDATA "localtex\models")
    }
    return (Join-Path $HOME "AppData\Local\localtex\models")
}

function Test-ShipRoot([string]$Dir) {
    $opendoc = Test-Path -LiteralPath (Join-Path $Dir "opendoc\layout.onnx") -PathType Leaf
    $hand = Test-Path -LiteralPath (Join-Path $Dir "handwriting\encoder.onnx") -PathType Leaf
    return ($opendoc -or $hand)
}

function Copy-Ship([string]$Src, [string]$Dst) {
    New-Item -ItemType Directory -Force -Path $Dst | Out-Null
    foreach ($name in @("opendoc", "handwriting")) {
        $from = Join-Path $Src $name
        $to = Join-Path $Dst $name
        if (Test-Path -LiteralPath $to) {
            Remove-Item -LiteralPath $to -Recurse -Force
        }
        Copy-Item -LiteralPath $from -Destination $to -Recurse
    }
    $manifest = Join-Path $Src "manifest.json"
    if (Test-Path -LiteralPath $manifest -PathType Leaf) {
        Copy-Item -LiteralPath $manifest -Destination (Join-Path $Dst "manifest.json") -Force
    }
}

function Test-GhReady {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        return $false
    }
    if ($env:GH_TOKEN -or $env:GITHUB_TOKEN) {
        return $true
    }
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        gh auth status 2>$null | Out-Null
        return ($LASTEXITCODE -eq 0)
    } catch {
        return $false
    } finally {
        $ErrorActionPreference = $prev
    }
}

function Get-RemoteFile([string]$Url, [string]$OutFile) {
    $ProgressPreference = "SilentlyContinue"
    $n = 0
    while ($true) {
        $n++
        try {
            Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
            return
        } catch {
            if ($n -ge 3) { throw }
            Start-Sleep -Seconds 1
        }
    }
}

function Assert-Sha256Sums([string]$Dir, [string]$SumsFile) {
    Get-Content -LiteralPath $SumsFile | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) { return }
        $parts = $line -split "\s+", 2
        if ($parts.Count -lt 2) { return }
        $want = $parts[0].ToLowerInvariant()
        $name = $parts[1].Trim().TrimStart("*")
        $path = Join-Path $Dir $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return }
        $got = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($got -ne $want) {
            throw "localtex: checksum mismatch for $name"
        }
        Write-Host "${name}: OK"
    }
}

if ($Dest) {
    # keep
} elseif ($env:LOCALTEX_MODELS) {
    $Dest = $env:LOCALTEX_MODELS
} else {
    $Dest = Get-DefaultModelsDir
}

if (Test-ShipRoot $Dest) {
    Write-Host "localtex: models already in $Dest"
    exit 0
}

$candidates = @()
if ($env:LOCALTEX_MODELS -and ($env:LOCALTEX_MODELS -ne $Dest)) {
    $candidates += $env:LOCALTEX_MODELS
}
$defaultDir = Get-DefaultModelsDir
if ($defaultDir -ne $Dest) {
    $candidates += $defaultDir
}
foreach ($src in $candidates) {
    if (Test-ShipRoot $src) {
        Write-Host "localtex: copying models from $src -> $Dest"
        Copy-Ship $src $Dest
        exit 0
    }
}

$tar = Get-Command tar -ErrorAction SilentlyContinue
if (-not $tar) {
    throw "localtex: missing 'tar'"
}

New-Item -ItemType Directory -Force -Path $Dest | Out-Null
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("localtex-models-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
    Write-Host "localtex: downloading $Repo@$Tag -> $Dest"
    if (Test-GhReady) {
        gh release download $Tag --repo $Repo --dir $work `
            --pattern $OpenDocTar --pattern $HandwritingTar `
            --pattern $Sums --pattern "manifest.json"
        if ($LASTEXITCODE -ne 0) { throw "localtex: gh release download failed" }
    } else {
        Write-Host "localtex: gh not authenticated; trying public HTTPS (fails if the repo is private)"
        $base = "https://github.com/${Repo}/releases/download/${Tag}"
        Get-RemoteFile "$base/$OpenDocTar" (Join-Path $work $OpenDocTar)
        Get-RemoteFile "$base/$HandwritingTar" (Join-Path $work $HandwritingTar)
        Get-RemoteFile "$base/$Sums" (Join-Path $work $Sums)
        Get-RemoteFile "$base/manifest.json" (Join-Path $work "manifest.json")
    }

    Assert-Sha256Sums $work (Join-Path $work $Sums)

    $extract = Join-Path $work "extract"
    New-Item -ItemType Directory -Force -Path $extract | Out-Null
    & tar -xzf (Join-Path $work $OpenDocTar) -C $extract
    if ($LASTEXITCODE -ne 0) { throw "localtex: tar failed for $OpenDocTar" }
    & tar -xzf (Join-Path $work $HandwritingTar) -C $extract
    if ($LASTEXITCODE -ne 0) { throw "localtex: tar failed for $HandwritingTar" }

    $openSrc = Join-Path $extract $OpenDocDir
    $handSrc = Join-Path $extract $HandwritingDir
    if (-not (Test-Path -LiteralPath $openSrc -PathType Container)) {
        throw "localtex: missing $OpenDocDir in tarball"
    }
    if (-not (Test-Path -LiteralPath $handSrc -PathType Container)) {
        throw "localtex: missing $HandwritingDir in tarball"
    }

    foreach ($name in @("opendoc", "handwriting", "current", "ship", $OpenDocDir, $HandwritingDir)) {
        $old = Join-Path $Dest $name
        if (Test-Path -LiteralPath $old) {
            Remove-Item -LiteralPath $old -Recurse -Force
        }
    }
    foreach ($name in @(
            "layout.onnx", "encoder.onnx", "decoder.onnx", "unirec_tokenizer_mapping.json",
            "inktex-encoder.onnx", "inktex-decoder-step.onnx", "inktex-vocab.json"
        )) {
        $old = Join-Path $Dest $name
        if (Test-Path -LiteralPath $old -PathType Leaf) {
            Remove-Item -LiteralPath $old -Force
        }
    }

    Move-Item -LiteralPath $openSrc -Destination (Join-Path $Dest "opendoc")
    Move-Item -LiteralPath $handSrc -Destination (Join-Path $Dest "handwriting")
    Copy-Item -LiteralPath (Join-Path $work "manifest.json") -Destination (Join-Path $Dest "manifest.json") -Force
    Copy-Item -LiteralPath (Join-Path $work $Sums) -Destination (Join-Path $Dest $Sums) -Force
} finally {
    if (Test-Path -LiteralPath $work) {
        Remove-Item -LiteralPath $work -Recurse -Force
    }
}

Write-Host "localtex: OpenDoc ready in $Dest\opendoc ($OpenDocDir)"
Write-Host "localtex: handwriting ready in $Dest\handwriting ($HandwritingDir)"
exit 0
