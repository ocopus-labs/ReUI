# Measures what ReUI costs, for the README's Performance table: CPU (share of the whole
# machine, all cores) and private memory of the hub and the island, sampled once a second.
# Runs a release build sandboxed (REUI_CONFIG_DIR in %TEMP%, its own pipe and files), so it
# runs beside an installed ReUI and never touches its data; modules that would react to the
# user's own input (clipboard, volume keys, devices, mic and camera, agents' hooks) are off,
# and nothing is played, typed or clicked. Three scenes:
#   rest   nothing happening (the idle island)
#   music  a track playing, card closed (the demo player: live bars, synthetic levels)
#   stats  the System stats card open (it reads the counters once a second while open)
#   pwsh -File tools\perf-sample.ps1 [-Seconds 60] [-NoBuild]
param(
    [int]$Seconds = 60,
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
if (-not $NoBuild) {
    cargo build --release -p reui-hub -p reui-island -p reui-bridge --manifest-path "$root\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
$bin = Join-Path $root "target\release"
$cores = [Environment]::ProcessorCount
$sandbox = Join-Path $env:TEMP "reui-perf-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force "$sandbox\roaming" | Out-Null
# The island always shows (so "rest" is the resting pill on screen), and no first run.
Set-Content "$sandbox\roaming\settings.json" '{ "visibility": "always" }'
$quiet = @("--no-updates", "--no-onboarding", "--no-agents", "--no-clipboard", "--no-osd", "--no-devices", "--no-privacy", "--no-media")

function Ours { Get-Process reui-hub, reui-island -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$bin\*" } }

function Start-Hub([string[]]$extra) {
    $env:REUI_CONFIG_DIR = $sandbox
    Start-Process -FilePath "$bin\reui-hub.exe" -ArgumentList ($quiet + $extra) | Out-Null
    Remove-Item Env:REUI_CONFIG_DIR
    for ($i = 0; $i -lt 80 -and @(Ours).Count -lt 2; $i++) { Start-Sleep -Milliseconds 250 }
    if (@(Ours).Count -lt 2) { throw "the sandboxed hub and island didn't start" }
}

function Stop-Hub { Ours | Stop-Process -Force; Start-Sleep -Milliseconds 500 }

# CPU % (of the machine) over the window and the mean private memory, per process.
function Measure-Scene([string]$name, [int]$Seconds = $script:Seconds) {
    Start-Sleep -Seconds 5  # let start-up and the scene's first frames settle
    $procs = @(Ours)
    $start = @{}; foreach ($p in $procs) { $p.Refresh(); $start[$p.Id] = $p.TotalProcessorTime }
    $mem = @{}; foreach ($p in $procs) { $mem[$p.Id] = New-Object System.Collections.Generic.List[double] }
    # How busy the rest of the machine was meanwhile, so a noisy run can be told apart.
    $load = New-Object System.Collections.Generic.List[double]
    $clock = [Diagnostics.Stopwatch]::StartNew()
    for ($s = 0; $s -lt $Seconds; $s++) {
        Start-Sleep -Seconds 1
        foreach ($p in $procs) { $p.Refresh(); $mem[$p.Id].Add($p.PrivateMemorySize64) }
        if ($s % 5 -eq 0) { $load.Add((Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average) }
    }
    $wall = $clock.Elapsed.TotalSeconds
    foreach ($p in $procs) {
        $p.Refresh()
        $cpu = ($p.TotalProcessorTime - $start[$p.Id]).TotalSeconds / $wall / $cores * 100
        [pscustomobject]@{
            Scene = $name
            Process = $p.ProcessName
            "CPU %" = "{0:N3}" -f $cpu
            "Private MB" = [math]::Round(($mem[$p.Id] | Measure-Object -Average).Average / 1MB, 1)
            "Machine %" = [math]::Round(($load | Measure-Object -Average).Average)
        }
    }
}

$results = @()
try {
    Start-Hub @(); $results += Measure-Scene "rest"; Stop-Hub
    Start-Hub @("--demo-music"); $results += Measure-Scene "music"; Stop-Hub
    Start-Hub @()
    $env:REUI_CONFIG_DIR = $sandbox; & "$bin\reui-bridge.exe" stats; Remove-Item Env:REUI_CONFIG_DIR
    $results += Measure-Scene "stats" -Seconds ([math]::Min($Seconds, 50))
} finally {
    Stop-Hub
    Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
}
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
"$(Get-Date -Format 'yyyy-MM-dd') · $cpu · $cores logical cores · $Seconds s per scene"
$results | Format-Table -AutoSize
