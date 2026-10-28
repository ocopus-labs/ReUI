# Opens a sandboxed Settings window, drives it with messages posted to its own hwnd only
# (never global input), and captures the live window (PrintWindow, full content) after each
# step, to check what the swap chain really shows.
#
#   powershell -File crates\reui-settings\perf\live.ps1 [-Exe target\release\reui-settings.exe] [-Out target\shots\v2\live]
param(
    [string]$Exe = "target\release\reui-settings.exe",
    [string]$Out = "target\shots\v2\live"
)
$ErrorActionPreference = "Stop"
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class Live {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l, t, r, b; }
    public static IntPtr Find() { return FindWindowW("ReUI.Settings.Sandbox", null); }
    static IntPtr Pt(int x, int y) { return (IntPtr)(((y & 0xffff) << 16) | (x & 0xffff)); }
    public static float Scale(IntPtr h) { return GetDpiForWindow(h) / 96f; }
    /// A click at (x, y) in DIPs.
    public static void Click(IntPtr h, float x, float y) {
        float s = Scale(h); var p = Pt((int)(x * s), (int)(y * s));
        PostMessageW(h, 0x200, IntPtr.Zero, p); PostMessageW(h, 0x201, (IntPtr)1, p); PostMessageW(h, 0x202, IntPtr.Zero, p);
    }
    public static void Move(IntPtr h, float x, float y) { float s = Scale(h); PostMessageW(h, 0x200, IntPtr.Zero, Pt((int)(x * s), (int)(y * s))); }
    public static void Key(IntPtr h, int vk) { PostMessageW(h, 0x100, (IntPtr)vk, IntPtr.Zero); }
    public static void Char(IntPtr h, char c) { PostMessageW(h, 0x102, (IntPtr)c, IntPtr.Zero); }
    public static void Wheel(IntPtr h, int d) { PostMessageW(h, 0x20A, (IntPtr)((d & 0xffff) << 16), IntPtr.Zero); }
    public static void Shot(IntPtr h, string path) {
        RECT r; GetWindowRect(h, out r);
        using (var bmp = new Bitmap(r.r - r.l, r.b - r.t)) {
            using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); PrintWindow(h, dc, 2); g.ReleaseHdc(dc); }
            bmp.Save(path, ImageFormat.Png);
        }
    }
}
"@
$Out = [IO.Path]::GetFullPath($Out)
New-Item -ItemType Directory -Force $Out | Out-Null
$sandbox = Join-Path $env:TEMP "reui-settings-live"
Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
$env:REUI_CONFIG_DIR = $sandbox
$proc = Start-Process -FilePath $Exe -ArgumentList "--page", "general" -PassThru
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 100 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $h = [Live]::Find() }
Start-Sleep -Milliseconds 900
[Live]::Shot($h, "$Out\1-open.png")
[Live]::Click($h, 120, 266); Start-Sleep -Milliseconds 800            # Now Playing in the sidebar
[Live]::Move($h, 120, 356); Start-Sleep -Milliseconds 300             # hover Volume & brightness
[Live]::Shot($h, "$Out\2-now-playing-hover.png")
[Live]::Wheel($h, -360); Start-Sleep -Milliseconds 600                 # three notches down
[Live]::Shot($h, "$Out\3-scrolled.png")
[Live]::Key($h, 0x46 -bor 0); Start-Sleep -Milliseconds 100
[Live]::Click($h, 120, 26); "dev" -split "" | Where-Object { $_ } | ForEach-Object { [Live]::Char($h, [char]$_) }; Start-Sleep -Milliseconds 400
[Live]::Shot($h, "$Out\4-search.png")
[Live]::Key($h, 0x0D); Start-Sleep -Milliseconds 800                   # Enter: first match
[Live]::Shot($h, "$Out\5-search-enter.png")
$proc.Refresh(); "process alive: " + (-not $proc.HasExited)
Stop-Process $proc
