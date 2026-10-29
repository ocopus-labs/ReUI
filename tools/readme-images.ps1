# Renders the README's pictures into docs/images: each island moment on its own stage,
# drawn offscreen by Settings' shot mode (`reui-settings --shots <dir> --stages`) at 200 %,
# on a flat backdrop colour instead of anyone's wallpaper. Runs Settings sandboxed
# (REUI_CONFIG_DIR in %TEMP%), so it reads default settings, touches no real files, and
# runs beside an open Settings window. Nothing is captured from the screen.
#   pwsh -File tools\readme-images.ps1 [-Backdrop "#5E6670"]
param(
    [string]$Out = "$PSScriptRoot\..\docs\images",
    [string]$Backdrop = "#5E6670"
)

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."

cargo build -p reui-settings --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw "build failed" }
$exe = Join-Path $root "target\debug\reui-settings.exe"

$sandbox = Join-Path $env:TEMP "reui-readme-$([guid]::NewGuid().ToString('N'))"
$shots = Join-Path $sandbox "shots"
New-Item -ItemType Directory -Force $shots, $Out | Out-Null
$env:REUI_CONFIG_DIR = Join-Path $sandbox "config"
$p = Start-Process -FilePath $exe -ArgumentList "--shots", $shots, "--stages", "--backdrop-color", $Backdrop -Wait -PassThru -NoNewWindow
Remove-Item Env:REUI_CONFIG_DIR
if ($p.ExitCode -ne 0) { throw "reui-settings --shots exited $($p.ExitCode)" }

# README name <- stage shot.
$pick = [ordered]@{
    "now-playing"       = "stage-compact"
    "clipboard"         = "stage-clipboard"
    "volume"            = "stage-meter"
    "devices"           = "stage-devices"
    "timers"            = "stage-timer"
    "agents"            = "stage-permission"
    "appearance-light"  = "stage-timer-light"
    "appearance-notch"  = "stage-timer-notch"
}
foreach ($name in $pick.Keys) {
    Copy-Item (Join-Path $shots "$($pick[$name]).png") (Join-Path $Out "$name.png") -Force
}

# The hero: the Now Playing card with more of the screen's top either side (the backdrop
# is one flat colour, so widening it is exact).
$card = [System.Drawing.Image]::FromFile((Join-Path $shots "stage-music.png"))
$width = 1920
$hero = New-Object System.Drawing.Bitmap $width, $card.Height
$g = [System.Drawing.Graphics]::FromImage($hero)
$under = ([System.Drawing.Bitmap]$card).GetPixel(0, $card.Height - 1)
$g.Clear($under)
$g.DrawImageUnscaled($card, [int](($width - $card.Width) / 2), 0)
$g.Dispose()
$card.Dispose()
$hero.Save((Join-Path $Out "hero.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$hero.Dispose()

Remove-Item -Recurse -Force $sandbox
Get-ChildItem $Out -Filter *.png | ForEach-Object { "{0,-22} {1,7:N0} bytes" -f $_.Name, $_.Length }
