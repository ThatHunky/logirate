# Generates assets\logirate.ico (16-256 px) and assets\logirate-256.png. Windows PowerShell 5.1 (System.Drawing).
Add-Type -AssemblyName System.Drawing
$here = $PSScriptRoot

function New-RoundedRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

function New-IconPng([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.ScaleTransform($size / 256.0, $size / 256.0)

    # tile
    $tile = New-RoundedRect 8 8 240 240 52
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 22, 27, 38))), $tile)

    # mouse body
    $white = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 240, 244, 250))
    $g.FillPath($white, (New-RoundedRect 52 50 98 156 46))
    # button split and wheel, cut out in the tile colour
    $cut = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 22, 27, 38))
    $g.FillRectangle($cut, 52, 112, 98, 8)
    $g.FillPath($cut, (New-RoundedRect 92 66 18 34 9))

    # rate bars, rising
    $teal = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 45, 212, 191))
    $g.FillPath($teal, (New-RoundedRect 166 150 18 50 7))
    $g.FillPath($teal, (New-RoundedRect 192 112 18 88 7))
    $g.FillPath($teal, (New-RoundedRect 218 70 18 130 7))

    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$pngs = foreach ($s in $sizes) { , (New-IconPng $s) }
[IO.File]::WriteAllBytes((Join-Path $here 'logirate-256.png'), $pngs[-1])

# ICO container with PNG-compressed entries
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ms
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $len = $pngs[$i].Length
    $w.Write([byte]($(if ($s -ge 256) { 0 } else { $s }))); $w.Write([byte]($(if ($s -ge 256) { 0 } else { $s })))
    $w.Write([byte]0); $w.Write([byte]0); $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$len); $w.Write([uint32]$offset)
    $offset += $len
}
foreach ($p in $pngs) { $w.Write($p) }
$w.Flush()
[IO.File]::WriteAllBytes((Join-Path $here 'logirate.ico'), $ms.ToArray())
Get-Item (Join-Path $here 'logirate.ico'), (Join-Path $here 'logirate-256.png') | Select-Object Name, Length
