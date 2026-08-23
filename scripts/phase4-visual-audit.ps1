param(
    [string]$Root = 'C:\Users\server\src\amazing'
)

Add-Type -AssemblyName System.Drawing

$audit = Join-Path $Root 'target\audit'
$amazingSource = [System.Drawing.Bitmap]::new((Join-Path $audit 'phase4.bmp'))
$referenceSource = [System.Drawing.Bitmap]::new((Join-Path $audit 'phase4-edge-reference.png'))
if ($amazingSource.Width -ne $referenceSource.Width -or
    $amazingSource.Height -ne $referenceSource.Height) {
    throw 'Phase 4 images have different dimensions'
}

$width = $amazingSource.Width
$height = $amazingSource.Height
$format = [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
$amazing = [System.Drawing.Bitmap]::new($width, $height, $format)
$reference = [System.Drawing.Bitmap]::new($width, $height, $format)
$ga = [System.Drawing.Graphics]::FromImage($amazing)
$gr = [System.Drawing.Graphics]::FromImage($reference)
$ga.DrawImageUnscaled($amazingSource, 0, 0)
$gr.DrawImageUnscaled($referenceSource, 0, 0)
$ga.Dispose()
$gr.Dispose()

$rect = [System.Drawing.Rectangle]::new(0, 0, $width, $height)
$aData = $amazing.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, $format)
$rData = $reference.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, $format)
$byteCount = [Math]::Abs($aData.Stride) * $height
$aBytes = [byte[]]::new($byteCount)
$rBytes = [byte[]]::new($byteCount)
[Runtime.InteropServices.Marshal]::Copy($aData.Scan0, $aBytes, 0, $byteCount)
[Runtime.InteropServices.Marshal]::Copy($rData.Scan0, $rBytes, 0, $byteCount)
$amazing.UnlockBits($aData)
$reference.UnlockBits($rData)

$diff = [System.Drawing.Bitmap]::new($width, $height, $format)
$dData = $diff.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, $format)
$dBytes = [byte[]]::new($byteCount)
$squared = 0.0
for ($offset = 0; $offset -lt $byteCount; $offset += 4) {
    for ($channel = 0; $channel -lt 3; $channel++) {
        $delta = [int]$aBytes[$offset + $channel] - [int]$rBytes[$offset + $channel]
        $squared += $delta * $delta
        $dBytes[$offset + $channel] = [byte][Math]::Abs($delta)
    }
    $dBytes[$offset + 3] = 255
}
[Runtime.InteropServices.Marshal]::Copy($dBytes, 0, $dData.Scan0, $byteCount)
$diff.UnlockBits($dData)

$rmse = [Math]::Sqrt($squared / ($width * $height * 3)) / 255.0
$png = [System.Drawing.Imaging.ImageFormat]::Png
$amazing.Save((Join-Path $audit 'phase4-amazing.png'), $png)
$reference.Save((Join-Path $audit 'phase4-edge-reference-final.png'), $png)
$diff.Save((Join-Path $audit 'phase4-absolute-diff.png'), $png)

$side = [System.Drawing.Bitmap]::new($width * 2, $height, $format)
$gs = [System.Drawing.Graphics]::FromImage($side)
$gs.Clear([System.Drawing.Color]::White)
$gs.DrawImageUnscaled($amazing, 0, 0)
$gs.DrawImageUnscaled($reference, $width, 0)
$gs.Dispose()
$side.Save((Join-Path $audit 'phase4-side-by-side.png'), $png)

'WIDTH={0}' -f $width
'HEIGHT={0}' -f $height
'RMSE_NORMALIZED={0:F6}' -f $rmse

$side.Dispose()
$diff.Dispose()
$amazing.Dispose()
$reference.Dispose()
$amazingSource.Dispose()
$referenceSource.Dispose()
