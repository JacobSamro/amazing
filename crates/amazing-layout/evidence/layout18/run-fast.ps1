param(
    [string]$Repository = 'C:\Users\server\src\wt\layout',
    [string]$ManifestPath = 'crates\amazing-layout\evidence\layout18\flex-grid-manifest.json',
    [string]$WptRoot = 'C:\Users\server\target\layout\wpt',
    [string]$OutputDirectory = 'C:\Users\server\target\layout\layout18-fast',
    [string]$ExecutablePath = 'C:\Users\server\target\layout\layout18-candidate.exe',
    [string]$ValidateDirectory = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Only the pixel loop changes. The repository runner still owns rendering,
# reference discovery, fuzzy limits, timeouts, classifications and JSON output.
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
public static class Layout18Pixels {
    public static int[] Compare(byte[] actual, byte[] reference, string path, int width, int height) {
        if (actual.Length != reference.Length || actual.Length != width * height * 3)
            throw new ArgumentException("PPM pixel buffer dimensions differ");
        byte[] diff = new byte[actual.Length];
        int count = 0, maximum = 0;
        for (int pixel = 0; pixel < actual.Length; pixel += 3) {
            bool changed = false;
            for (int channel = 0; channel < 3; channel++) {
                int delta = Math.Abs((int)actual[pixel + channel] - (int)reference[pixel + channel]);
                changed |= delta != 0;
                maximum = Math.Max(maximum, delta);
            }
            if (changed) count++;
            diff[pixel] = 255;
            diff[pixel + 1] = changed ? (byte)0 : (byte)255;
            diff[pixel + 2] = 255;
        }
        byte[] header = Encoding.ASCII.GetBytes("P6\n" + width + " " + height + "\n255\n");
        using (FileStream output = File.Create(path)) {
            output.Write(header, 0, header.Length);
            output.Write(diff, 0, diff.Length);
        }
        return new int[] { count, maximum };
    }
}
'@
$source = Get-Content (Join-Path $Repository 'scripts\run-render-wpt.ps1') -Raw
$rootLine = '$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))'
if (-not $source.Contains($rootLine)) { throw 'Repository runner root assignment changed' }
$source = $source.Replace($rootLine, ('$repositoryRoot = ''' + $Repository.Replace("'", "''") + ''''))
$marker = '$runName = '
$offset = $source.IndexOf($marker)
if ($offset -lt 0) { throw 'Repository runner entry marker changed' }
$override = @'
function Compare-Ppm {
    param([string]$ActualPath, [string]$ReferencePath, [string]$DiffPath,
          [int]$AllowedDifference, [int]$AllowedPixels)
    $actual = Read-Ppm $ActualPath
    $reference = Read-Ppm $ReferencePath
    if ($actual.width -ne $reference.width -or $actual.height -ne $reference.height) {
        return [pscustomobject]@{ matched=$false; differing_pixels=$actual.width * $actual.height;
            max_difference=255; detail='image dimensions differ' }
    }
    $result = [Layout18Pixels]::Compare($actual.pixels, $reference.pixels, $DiffPath, $actual.width, $actual.height)
    $differentPixels = $result[0]; $maximumDifference = $result[1]
    return [pscustomobject]@{
        matched=($maximumDifference -le $AllowedDifference -and $differentPixels -le $AllowedPixels)
        differing_pixels=$differentPixels; max_difference=$maximumDifference
        detail="maxDifference=$maximumDifference/$AllowedDifference; totalPixels=$differentPixels/$AllowedPixels"
    }
}
'@
if ($ValidateDirectory) {
    # Reuse both original functions, then redefine only Compare-Ppm. Compare
    # metrics, threshold decisions and exact diff-file hashes against the old
    # implementation on real saved render pairs, including mismatches.
    $validation = @'
$cases = @(Get-ChildItem $ValidateDirectory -Directory | Where-Object {
    Test-Path (Join-Path $_.FullName 'diff.ppm')
} | Where-Object {
    (Get-FileHash (Join-Path $_.FullName 'actual.ppm')).Hash -ne
    (Get-FileHash (Join-Path $_.FullName 'reference.ppm')).Hash
} | Select-Object -First 4)
if ($cases.Count -lt 2) { throw 'Need saved original comparisons for parity validation' }
foreach ($case in $cases) {
    $actual = Join-Path $case.FullName 'actual.ppm'
    $reference = Join-Path $case.FullName 'reference.ppm'
    $temporary = Join-Path $case.FullName 'fast-parity.ppm'
    $old = Compare-Ppm $actual $reference $temporary 3 7
    $oldHash = (Get-FileHash $temporary).Hash
    $a = Read-Ppm $actual; $r = Read-Ppm $reference
    $metrics = [Layout18Pixels]::Compare($a.pixels, $r.pixels, $temporary, $a.width, $a.height)
    if ($old.differing_pixels -ne $metrics[0] -or $old.max_difference -ne $metrics[1] -or
        $old.matched -ne ($metrics[1] -le 3 -and $metrics[0] -le 7) -or
        $oldHash -ne (Get-FileHash $temporary).Hash -or
        $oldHash -ne (Get-FileHash (Join-Path $case.FullName 'diff.ppm')).Hash) {
        throw "Pixel comparison parity failed: $($case.Name)"
    }
    Remove-Item $temporary
    Write-Output "PARITY $($case.Name) pixels=$($metrics[0]) maximum=$($metrics[1]) sha256=$oldHash"
}
Write-Output "PARITY PASS: $($cases.Count) original/native comparisons"
'@
    # The runner prelude validates manifest/pin and defines its original
    # functions. No new render cases or output directories are created here.
    $source = $source.Substring(0, $offset) + $validation
} else {
    $source = $source.Insert($offset, $override + "`n")
}
& ([ScriptBlock]::Create($source)) -ManifestPath $ManifestPath -WptRoot $WptRoot `
    -OutputDirectory $OutputDirectory -ExecutablePath $ExecutablePath `
    -ViewportWidth 800 -ViewportHeight 600 -TimeoutSeconds 20 -SkipBuild -SkipFetch
