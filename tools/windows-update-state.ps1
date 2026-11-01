# Saves what Windows Update says about restarts, for docs/PRD-update-restart.md (M1).
# Read-only, no admin. Run it at each step and diff the files:
#   1. before updates install       pwsh tools/windows-update-state.ps1 -Name before
#   2. once "Restart required"      ... -Name pending
#   3. after "Schedule the restart" ... -Name scheduled
#   4. as the restart nears         ... -Name soon
# Files go to %TEMP%\reui-wu-state\<name>.json; -Diff a,b compares two of them.
param([string]$Name = (Get-Date -Format 'HHmmss'), [string[]]$Diff)

$dir = Join-Path $env:TEMP 'reui-wu-state'
New-Item -ItemType Directory -Force $dir | Out-Null

if ($Diff) {
    $a = Get-Content (Join-Path $dir "$($Diff[0]).json") -Raw | ConvertFrom-Json -AsHashtable
    $b = Get-Content (Join-Path $dir "$($Diff[1]).json") -Raw | ConvertFrom-Json -AsHashtable
    foreach ($k in ($a.Keys + $b.Keys | Sort-Object -Unique)) {
        if ("$($a[$k])" -ne "$($b[$k])") { "{0}`n    {1} -> {2}" -f $k, $a[$k], $b[$k] }
    }
    return
}

$roots = 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending'
$state = [ordered]@{}
foreach ($root in $roots) {
    $keys = @(Get-Item $root -ErrorAction SilentlyContinue) + @(Get-ChildItem $root -Recurse -ErrorAction SilentlyContinue)
    foreach ($key in $keys) {
        $path = $key.Name -replace '^HKEY_LOCAL_MACHINE', 'HKLM'
        $state["$path\"] = '(key)'
        foreach ($value in $key.GetValueNames()) {
            $data = $key.GetValue($value)
            if ($data -is [byte[]]) { $data = [Convert]::ToBase64String($data) }
            elseif ($data -is [array]) { $data = $data -join ' | ' }
            # Unix-ms times, readable beside the number.
            if ($data -is [long] -and $data -gt 1500000000000 -and $data -lt 2500000000000) {
                $data = "$data ($([DateTimeOffset]::FromUnixTimeMilliseconds($data).LocalDateTime.ToString('yyyy-MM-dd HH:mm:ss')))"
            }
            $state["$path\$value"] = "$data"
        }
    }
}
try { $state['WUA\RebootRequired'] = "$((New-Object -ComObject Microsoft.Update.SystemInfo).RebootRequired)" } catch { $state['WUA\RebootRequired'] = "error: $_" }
$state['taken'] = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')

$file = Join-Path $dir "$Name.json"
$state | ConvertTo-Json | Set-Content $file
"saved $($state.Count) values to $file"
