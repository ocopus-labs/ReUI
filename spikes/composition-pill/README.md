# Phase 0a spike: native composition pill

A dynamic-island pill drawn with Windows.UI.Composition from Rust, to test the native-first renderer decision in `PLAN.md`.

```
cargo build --release
target\release\composition-pill.exe [--expanded] [--paused] [--demo]
```

Hover the pill to peek, click to expand, use the transport buttons (prev/next change the fake track, play/pause stops the bars), right-click to quit. `--demo` toggles expanded/compact every 1.6 s.

`tools\shot.ps1` screenshots the top of the primary monitor, and `tools\measure.ps1` samples the pill's CPU and memory plus dwm.exe CPU and 3D GPU.

## How it works

- Win32 popup window with `WS_EX_TOPMOST | TOOLWINDOW | NOACTIVATE | NOREDIRECTIONBITMAP | LAYERED | TRANSPARENT`, hosted via `DesktopWindowTarget`.
- The pill's width and height are two scalars in a `CompositionPropertySet`, animated with `SpringScalarNaturalMotionAnimation`. (Vector springs need an `IReference<Vector2>`, which windows-rs can't build easily; two scalar springs also allow different damping per axis.)
- Everything else is an `ExpressionAnimation` over those two values: pill size, centring, rounded clip and corner radius, the cross-fade and scale between compact and expanded content. A morph is set up once and then runs entirely in DWM.
- Content is drawn with Direct2D/DirectWrite into two `CompositionDrawingSurface`s, and redrawn only when the content changes.
- Click-through: a `WH_MOUSE_LL` hook clears `WS_EX_TRANSPARENT` only while the cursor is over the pill. `WM_MOUSEACTIVATE → MA_NOACTIVATE` means clicks never take focus.
- Fullscreen hide: a `EVENT_SYSTEM_FOREGROUND` WinEvent hook plus `SHQueryUserNotificationState`.

## Results (Oct 27, 2026; RTX 3060, one 199 Hz monitor, 12 logical cores)

| Mode | Pill CPU | Pill memory (working set / private) | dwm.exe CPU (% of a core) | dwm.exe 3D GPU |
| --- | --- | --- | --- | --- |
| No pill (baseline, two runs) | — | — | 2.5–4.9 % | 0.7–1.0 % |
| Pill idle, paused | ≈ 0 (below timer resolution) | 47 / 57 MB | 2.9 % | 1.0 % |
| Pill idle, bars looping | ≈ 0 | 48 / 58 MB | 19–30 % | 0.9–6 % |
| Morph every 1.6 s, bars paused | ≈ 0 | 52 / 58 MB | 16.5 % | 2.7 % |

What this tells us:

1. **The pill process costs essentially nothing.** Its CPU is below what the process timer can resolve, even while morphing.
2. **An idle, static pill doesn't change DWM's load at all.**
3. **Any running animation makes DWM recompose every frame**, which costs about 15–30 % of one core on this 199 Hz display. That's inherent to the desktop compositor, so a web renderer would pay it too, plus its own rendering. Rule for the product: **no perpetual animations**. Morphs are short. The now-playing bars run only while the pill is expanded or hovered, or update at a low rate. Prefer static indicators.
4. **Memory is about 57 MB private**, higher than hoped, though about a third of WebView2's floor. It is likely the D3D11 device and GPU driver; to investigate (WARP vs hardware device, trimming the working set after startup).

Not covered yet: DPI changes and multi-monitor (scale is read once at start), F11-style fullscreen without a focus change, frosted-glass backdrop, a real media source.
