# Drives a fake Claude Code session through every state via reui-bridge and captures the
# island (compact and expanded) into one review sheet. Needs the hub running.
#   pwsh -File tools\claude-states.ps1 [-Out target\claude-states.png]
param([string]$Out = "$PSScriptRoot\..\target\claude-states.png")

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type -Namespace ReUI -Name Win -MemberDefinition @'
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
[DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
public struct RECT { public int Left, Top, Right, Bottom; }
'@
[ReUI.Win]::SetProcessDPIAware() | Out-Null
$bridge = Join-Path $PSScriptRoot "..\target\release\reui-bridge.exe"
$h = [ReUI.Win]::FindWindowW("ReUI.Island", "ReUI")
if ($h -eq [IntPtr]::Zero) { throw "island not running; start target\release\reui-hub.exe first" }
$r = New-Object ReUI.Win+RECT; [ReUI.Win]::GetClientRect($h, [ref]$r) | Out-Null
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$W = $r.Right; $x0 = $screen.X + [int](($screen.Width - $W) / 2)

function Send($json) { $json | & $bridge claude; Start-Sleep -Milliseconds 500 }
function Event($name, $extra = "") {
    $body = '"session_id":"review","cwd":"D:\\ReUI","transcript_path":"C:\\none.jsonl","hook_event_name":"' + $name + '"'
    if ($extra) { $body += ",$extra" }
    Send "{$body}"
}
function Click($x, $y) { [ReUI.Win]::PostMessageW($h, 0x0202, [IntPtr]::Zero, [IntPtr](($y -shl 16) -bor $x)) | Out-Null; Start-Sleep -Milliseconds 700 }
function Collapse { [ReUI.Win]::PostMessageW($h, 0x0113, [IntPtr]1, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 600 }
$shots = New-Object System.Collections.ArrayList
function Shot($label, $height) {
    $bmp = New-Object System.Drawing.Bitmap $W, $height
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($x0, $screen.Y, 0, 0, $bmp.Size); $g.Dispose()
    [void]$shots.Add(@($label, $bmp))
}
function Both($label) { Shot "$label - compact" 56; Click ([int]($W / 2)) 20; Shot "$label - expanded" 150; Collapse }

Event "UserPromptSubmit" '"prompt":"Fix the flaky island hover test and run the suite"'; Both "Thinking"
Event "PreToolUse" '"tool_name":"Bash","tool_input":{"command":"cargo test"}'; Both "Running a tool"
Event "SubagentStart"; Event "SubagentStart"; Event "PostToolUse" '"tool_name":"Bash"'; Both "Thinking + subagents"
Event "SubagentStop"; Event "SubagentStop"
Event "PreCompact" '"trigger":"auto"'; Both "Compacting"
Event "PostCompact"
Event "PreToolUse" '"tool_name":"AskUserQuestion"'; Both "Asks you something"
$perm = '{"session_id":"review","cwd":"D:\\ReUI","hook_event_name":"PermissionRequest","tool_name":"Edit","tool_input":{"file_path":"D:\\ReUI\\crates\\reui-island\\src\\pill.rs"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Edit"}],"behavior":"allow","destination":"session"}]}'
$job = Start-Job { param($b, $j) $j | & $b claude } -ArgumentList $bridge, $perm
Start-Sleep -Milliseconds 1200; Shot "Permission request (opens itself)" 180
Collapse; Shot "Permission request - folded" 56
Stop-Job $job; Remove-Job $job -Force; Start-Sleep -Milliseconds 700
Event "Stop"; Both "Done"
Start-Sleep -Seconds 6; Shot "6 s after done (should be gone)" 56
Event "UserPromptSubmit" '"prompt":"Ship it"'; Event "StopFailure" '"error":"API rate limit reached"'; Both "Failed"
Event "SessionEnd" '"reason":"other"'

$scale = 2; $labelH = 22
$total = ($shots | ForEach-Object { $_[1].Height * $scale + $labelH } | Measure-Object -Sum).Sum
$sheet = New-Object System.Drawing.Bitmap ($W * $scale), $total
$g = [System.Drawing.Graphics]::FromImage($sheet); $g.Clear([System.Drawing.Color]::FromArgb(24, 24, 24))
$g.InterpolationMode = 'NearestNeighbor'; $g.PixelOffsetMode = 'Half'
$font = New-Object System.Drawing.Font "Segoe UI", 11; $y = 0
foreach ($s in $shots) {
    $g.DrawString($s[0], $font, [System.Drawing.Brushes]::Gray, 6, $y + 2); $y += $labelH
    $g.DrawImage($s[1], (New-Object System.Drawing.Rectangle 0, $y, ($W * $scale), ($s[1].Height * $scale))); $y += $s[1].Height * $scale
    $s[1].Dispose()
}
$sheet.Save([System.IO.Path]::GetFullPath($Out)); "saved $Out"
