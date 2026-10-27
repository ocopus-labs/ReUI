# Samples CPU and memory of the pill process, plus CPU and 3D-GPU use of dwm.exe.
# Composition animations run inside DWM, so dwm.exe is part of the pill's real cost:
# compare a run with the pill idle against one with --demo, and against no pill at all.
# DWM's own process times aren't readable without admin, so it is sampled through
# performance counters (CPU % where 100 = one full core).
param([int]$Seconds = 20, [string]$Name = "composition-pill")

$cores = [Environment]::ProcessorCount
$pill = Get-Process -Name $Name -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $pill) { Write-Warning "$Name is not running; measuring dwm.exe only (baseline)." }
$dwmPid = (Get-Process dwm | Select-Object -First 1).Id
$pillCpu0 = if ($pill) { $pill.TotalProcessorTime.TotalMilliseconds } else { 0 }

$cpu = @(); $gpu = @()
for ($i = 0; $i -lt $Seconds; $i++) {
    $cpu += (Get-Counter '\Process(dwm)\% Processor Time').CounterSamples[0].CookedValue
    $samples = (Get-Counter "\GPU Engine(*)\Utilization Percentage" -ErrorAction SilentlyContinue).CounterSamples
    $gpu += ($samples | Where-Object { $_.InstanceName -like "pid_$($dwmPid)_*engtype_3d" } | Measure-Object CookedValue -Sum).Sum
}

if ($pill) {
    $pill.Refresh()
    $pillCpu = ($pill.TotalProcessorTime.TotalMilliseconds - $pillCpu0) / ($Seconds * 1000) * 100
    "{0,-18} CPU {1,6:N3} % of a core   working set {2,6:N1} MB   private {3,6:N1} MB" -f $Name, $pillCpu, ($pill.WorkingSet64 / 1MB), ($pill.PrivateMemorySize64 / 1MB)
}
"{0,-18} CPU {1,6:N2} % of a core   3D GPU {2,6:N2} %   (avg of {3} one-second samples, {4} logical cores)" -f "dwm.exe", ($cpu | Measure-Object -Average).Average, ($gpu | Measure-Object -Average).Average, $Seconds, $cores
