# Regenerate the Android launcher icons from packaging/logo.png.
#
# Run from anywhere on Windows (System.Drawing does the resizing):
#   .\mobile\android\scripts\make-icons.ps1
#
# Writes legacy mipmaps, adaptive foregrounds, and the in-app header logo.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$root = Resolve-Path (Join-Path $PSScriptRoot "../../..")
$sourcePath = Join-Path $root "packaging/logo.png"
$res = Join-Path $root "mobile/android/app/src/main/res"
$source = [System.Drawing.Bitmap]::FromFile($sourcePath)

function New-Icon([int]$size, [string]$path, [switch]$Circle, [double]$Inset) {
    $bitmap = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([System.Drawing.Color]::Transparent)
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias

    $scale = 1.0 - [Math]::Max(0.0, $Inset)
    $box = [int][Math]::Round($size * $scale)
    $offset = [int][Math]::Round(($size - $box) / 2)

    if ($Circle) {
        $shape = New-Object System.Drawing.Drawing2D.GraphicsPath
        $shape.AddEllipse(0, 0, $size, $size)
        $graphics.SetClip($shape)
        $back = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 11, 11, 19))
        $graphics.FillPath($back, $shape)
        $back.Dispose()
    }

    $graphics.DrawImage($source, $offset, $offset, $box, $box)
    $graphics.Dispose()
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
}

# density, legacy size, adaptive foreground size (108dp)
$densities = @(
    @("mdpi", 48, 108),
    @("hdpi", 72, 162),
    @("xhdpi", 96, 216),
    @("xxhdpi", 144, 324),
    @("xxxhdpi", 192, 432)
)

foreach ($density in $densities) {
    $name = $density[0]
    $legacy = $density[1]
    $adaptive = $density[2]
    $dir = Join-Path $res "mipmap-$name"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null

    New-Icon -size $legacy -path (Join-Path $dir "ic_launcher.png")
    New-Icon -size $legacy -path (Join-Path $dir "ic_launcher_round.png") -Circle
    # 72dp of a 108dp adaptive canvas, so the art stays inside the safe zone.
    New-Icon -size $adaptive -path (Join-Path $dir "ic_launcher_foreground.png") -Inset (1.0 / 3.0)
}

$drawable = Join-Path $res "drawable-nodpi"
New-Item -ItemType Directory -Force -Path $drawable | Out-Null
New-Icon -size 192 -path (Join-Path $drawable "app_logo.png")

$source.Dispose()
Write-Output "Launcher icons written to $res"
