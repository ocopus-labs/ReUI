# Draws the ReUI logo at every icon size and packs design/logo/reui.ico (+ a PNG per size).
# The mark: a light screen (rounded tile) with the black island at its top and the island's
# detached bubble beside it in the default accent. Each size is drawn on its own pixel grid
# (shapes snapped to whole pixels), so 16 px in the tray stays crisp instead of being a
# blurred downscale of 256. Colours are DESIGN tokens: settings.color.mark / mark-low,
# color.system.blue.
#   pwsh -File tools\logo.ps1
param([string]$Out = "$PSScriptRoot\..\design\logo")

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Out | Out-Null

$mark = [System.Drawing.ColorTranslator]::FromHtml("#F2F2F7")
$markLow = [System.Drawing.ColorTranslator]::FromHtml("#C7C7CC")
$accent = [System.Drawing.ColorTranslator]::FromHtml("#0A84FF")

function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = [Math]::Min(2 * $r, [Math]::Min($w, $h))
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

function Draw([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = "AntiAlias"
    $g.PixelOffsetMode = "Half"
    $g.Clear([System.Drawing.Color]::Transparent)

    # The tile fills small sizes edge to edge; larger ones keep Windows' icon margin.
    $pad = if ($s -ge 32) { [Math]::Round($s / 16) } else { 0 }
    $t = $s - 2 * $pad
    $radius = $t * 0.225
    $tile = RoundRect $pad $pad $t $t $radius
    $fill = New-Object System.Drawing.Drawing2D.LinearGradientBrush ([System.Drawing.PointF]::new(0, $pad)), ([System.Drawing.PointF]::new(0, $pad + $t)), $mark, $markLow
    $g.FillPath($fill, $tile)
    # A hairline keeps the light tile's edge on white backgrounds (Explorer, Start).
    $edge = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(36, 0, 0, 0)), ([Math]::Max(1.0, $s / 128))
    $inner = RoundRect ($pad + $edge.Width / 2) ($pad + $edge.Width / 2) ($t - $edge.Width) ($t - $edge.Width) ($radius - $edge.Width / 2)
    $g.DrawPath($edge, $inner)

    # The island and its bubble, on whole pixels, centred together near the top.
    $h = [Math]::Max(3, [Math]::Round($t * 0.17))
    $w = [Math]::Round($t * 0.44)
    $gap = [Math]::Max(1, [Math]::Round($t * 0.05))
    $total = $w + $gap + $h
    $x = $pad + [Math]::Floor(($t - $total) / 2)
    $y = $pad + [Math]::Round($t * 0.15)
    $black = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::Black)
    $g.FillPath($black, (RoundRect $x $y $w $h ($h / 2)))
    $g.FillEllipse((New-Object System.Drawing.SolidBrush $accent), ($x + $w + $gap), $y, $h, $h)

    $g.Dispose()
    $bmp
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$images = foreach ($s in $sizes) {
    $bmp = Draw $s
    $path = Join-Path $Out "reui-$s.png"
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , [IO.File]::ReadAllBytes($path)
}

# ICO: header, one directory entry per size, then the PNGs.
$ms = New-Object IO.MemoryStream
$bw = New-Object IO.BinaryWriter $ms
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $len = $images[$i].Length
    $dim = if ($s -ge 256) { 0 } else { $s }
    $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]$len); $bw.Write([uint32]$offset)
    $offset += $len
}
foreach ($img in $images) { $bw.Write($img) }
$bw.Flush()
[IO.File]::WriteAllBytes((Join-Path $Out "reui.ico"), $ms.ToArray())
"Wrote $Out\reui.ico ($($sizes -join ', ') px)"
