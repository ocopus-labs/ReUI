# Measures reui-settings: CPU at rest, while scrolling and while hovering, with input
# posted to the Settings window only (never global input). Runs a sandboxed window
# (REUI_CONFIG_DIR) with --perf and prints the CPU figures and the window's own frame log.
#
#   pwsh -File crates\reui-settings\perf\harness.ps1 [-Exe target\release\reui-settings.exe] [-Seconds 6]
param(
    [string]$Exe = "target\release\reui-settings.exe",
    [int]$Seconds = 6,
    [string]$Page = "clipboard"
)
$ErrorActionPreference = "Stop"

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class Harness {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint p);
    [DllImport("winmm.dll")] static extern uint timeEndPeriod(uint p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l, t, r, b; }
    const uint WM_MOUSEMOVE = 0x200, WM_MOUSEWHEEL = 0x20A, WM_CLOSE = 0x10;

    public static IntPtr Find() { return FindWindowW("ReUI.Settings.Sandbox", null); }
    static IntPtr Pt(int x, int y) { return (IntPtr)(((y & 0xffff) << 16) | (x & 0xffff)); }

    /// Wheel notches (or touchpad-sized deltas) at ~120 Hz, down then up.
    public static void Scroll(IntPtr h, int ms, int delta) {
        timeBeginPeriod(1);
        RECT r; GetClientRect(h, out r);
        var end = Environment.TickCount + ms; int i = 0;
        while (Environment.TickCount < end) {
            int d = ((i / 40) % 2 == 0) ? -delta : delta;
            PostMessageW(h, WM_MOUSEMOVE, IntPtr.Zero, Pt(r.r / 2, r.b / 2));
            PostMessageW(h, WM_MOUSEWHEEL, (IntPtr)((d & 0xffff) << 16), Pt(0, 0));
            Thread.Sleep(8); i++;
        }
        timeEndPeriod(1);
    }

    /// The pointer sweeping down the nav and down the page's rows at ~120 Hz.
    public static void Hover(IntPtr h, int ms) {
        timeBeginPeriod(1);
        RECT r; GetClientRect(h, out r);
        float s = GetDpiForWindow(h) / 96f;
        var end = Environment.TickCount + ms; int i = 0;
        while (Environment.TickCount < end) {
            int step = i % 120;
            int x = (i / 120) % 2 == 0 ? (int)(110 * s) : (int)(r.r - 120 * s);
            int y = (int)((60 + step * 5) * s);
            PostMessageW(h, WM_MOUSEMOVE, IntPtr.Zero, Pt(x, y));
            Thread.Sleep(8); i++;
        }
        timeEndPeriod(1);
    }

    public static void Close(IntPtr h) { PostMessageW(h, WM_CLOSE, IntPtr.Zero, IntPtr.Zero); }
}
"@

$sandbox = Join-Path $env:TEMP "reui-settings-perf"
Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
$env:REUI_CONFIG_DIR = $sandbox
$proc = Start-Process -FilePath $Exe -ArgumentList "--perf", "--page", $Page -PassThru
$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 100 -and $hwnd -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $hwnd = [Harness]::Find() }
if ($hwnd -eq [IntPtr]::Zero) { throw "no window" }
Start-Sleep -Seconds 2

function Cpu([scriptblock]$work, [string]$name) {
    $proc.Refresh(); $t0 = $proc.TotalProcessorTime; $w0 = [Diagnostics.Stopwatch]::StartNew()
    & $work
    $proc.Refresh(); $t1 = $proc.TotalProcessorTime
    $pct = 100.0 * ($t1 - $t0).TotalMilliseconds / $w0.Elapsed.TotalMilliseconds
    "{0,-22} {1,6:N2} % of one core   ({2:N0} ms CPU in {3:N1} s)" -f $name, $pct, ($t1 - $t0).TotalMilliseconds, $w0.Elapsed.TotalSeconds
}

$ms = $Seconds * 1000
$log = Join-Path $sandbox "local\settings-perf.log"
Cpu { Start-Sleep -Seconds $Seconds } "at rest"
function Lines { (Get-Content $log -ErrorAction SilentlyContinue | Measure-Object -Line).Lines }
$mark1 = Lines
Cpu { [Harness]::Scroll($hwnd, $ms, 120) } "wheel notches"
$mark2 = Lines
Cpu { [Harness]::Scroll($hwnd, $ms, 15) } "touchpad deltas"
$mark3 = Lines
Cpu { [Harness]::Hover($hwnd, $ms) } "hover sweep"
$mark4 = Lines
Cpu { Start-Sleep -Seconds $Seconds } "at rest again"
$proc.Refresh()
"private memory: {0:N1} MB" -f ($proc.PrivateMemorySize64 / 1MB)
[Harness]::Close($hwnd)
Start-Sleep -Milliseconds 500

$lines = @(Get-Content $log -ErrorAction SilentlyContinue)
if ($lines.Count -eq 0) { return }
"`n-- frame log: wheel --";    $lines[$mark1..($mark2 - 1)] | Select-Object -Last 4
"-- frame log: touchpad --";   $lines[$mark2..($mark3 - 1)] | Select-Object -Last 4
"-- frame log: hover --";      $lines[$mark3..($mark4 - 1)] | Select-Object -Last 4
