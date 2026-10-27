<#
.SYNOPSIS
Builds ReUI's release assets, the same way the GitHub release workflow does.

.DESCRIPTION
Produces, in -OutDir:
  ReUI-Setup-<version>.exe   single-file per-user installer: the programs are embedded at
                             build time (REUI_PAYLOAD_DIR -> include_bytes!, see
                             crates/reui-setup/src/payload_gen.rs)
  ReUI-<version>-x64.zip     the programs + a small reui-setup.exe that installs the files
                             next to it, README, CHANGELOG (and LICENSE if present)
Then verifies the setup by extracting its payload (reui-setup --extract) and comparing
hashes. Nothing is installed.

.EXAMPLE
pwsh scripts/package.ps1 -OutDir $env:TEMP\reui-dist -TargetDir $env:TEMP\reui-target
#>
param(
    [string]$OutDir = "dist",
    # cargo --target-dir; defaults to the workspace's target folder.
    [string]$TargetDir = "",
    # Skip `cargo build --release --workspace` (reuse what's built).
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$programs = @("reui-hub.exe", "reui-island.exe", "reui-bridge.exe", "reui-settings.exe")

function Invoke-Cargo([string[]]$CargoArgs) {
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo $($CargoArgs -join ' ') failed ($LASTEXITCODE)" }
}

Push-Location $root
try {
    $meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = ($meta.packages | Where-Object name -eq "reui-hub").version
    $target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { $meta.target_directory }
    $bin = Join-Path $target "release"
    $targetArgs = @("--target-dir", $target)
    $out = (New-Item -ItemType Directory -Force $OutDir).FullName
    Write-Host "ReUI $version -> $out (target $target)"

    # 1. Everything, and a setup with nothing embedded (for the zip).
    Remove-Item Env:REUI_PAYLOAD_DIR -ErrorAction SilentlyContinue
    if (-not $SkipBuild) { Invoke-Cargo (@("build", "--release", "--workspace") + $targetArgs) }
    Invoke-Cargo (@("build", "--release", "-p", "reui-setup") + $targetArgs)
    foreach ($p in $programs + "reui-setup.exe") {
        if (-not (Test-Path (Join-Path $bin $p))) { throw "missing $bin\$p" }
    }

    # 2. The zip.
    $zipName = "ReUI-$version-x64"
    $stage = Join-Path $out $zipName
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
    New-Item -ItemType Directory $stage | Out-Null
    foreach ($p in $programs + "reui-setup.exe") { Copy-Item (Join-Path $bin $p) $stage }
    foreach ($doc in @("README.md", "CHANGELOG.md", "LICENSE", "LICENSE.md", "LICENSE.txt")) {
        if (Test-Path $doc) { Copy-Item $doc $stage }
    }
    $zip = Join-Path $out "$zipName.zip"
    Remove-Item $zip -ErrorAction SilentlyContinue
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip

    # 3. The single-file setup: rebuild reui-setup with the programs embedded.
    $payload = Join-Path $out "payload"
    Remove-Item -Recurse -Force $payload -ErrorAction SilentlyContinue
    New-Item -ItemType Directory $payload | Out-Null
    foreach ($p in $programs) { Copy-Item (Join-Path $bin $p) $payload }
    $env:REUI_PAYLOAD_DIR = $payload
    try { Invoke-Cargo (@("build", "--release", "-p", "reui-setup") + $targetArgs) }
    finally { Remove-Item Env:REUI_PAYLOAD_DIR -ErrorAction SilentlyContinue }
    $setup = Join-Path $out "ReUI-Setup-$version.exe"
    Copy-Item (Join-Path $bin "reui-setup.exe") $setup -Force

    # 4. Verify: the setup carries exactly these programs (extract only, no install).
    $check = Join-Path $out "verify"
    Remove-Item -Recurse -Force $check -ErrorAction SilentlyContinue
    $proc = Start-Process -FilePath $setup -ArgumentList @("--extract", "`"$check`"", "--quiet") -Wait -PassThru -WindowStyle Hidden
    if ($proc.ExitCode -ne 0) { throw "$setup --extract failed ($($proc.ExitCode))" }
    foreach ($p in $programs) {
        $want = (Get-FileHash (Join-Path $payload $p)).Hash
        $got = (Get-FileHash (Join-Path $check $p)).Hash
        if ($want -ne $got) { throw "embedded $p differs from the build" }
    }
    $zipped = (Get-ChildItem $stage).Name
    foreach ($p in $programs + "reui-setup.exe") { if ($zipped -notcontains $p) { throw "zip lacks $p" } }

    Remove-Item -Recurse -Force $payload, $check, $stage
    Get-Item $setup, $zip | ForEach-Object { "{0,-32} {1,8:N1} MB  {2}" -f $_.Name, ($_.Length / 1MB), (Get-FileHash $_).Hash }
}
finally {
    Pop-Location
}
