param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Stage,
    [Parameter(Mandatory)][string]$DriverRevision
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$OutputDirectory = [IO.Path]::GetFullPath((Join-Path (Get-Location) $OutputDirectory))
$Executable = [IO.Path]::GetFullPath((Join-Path (Get-Location) $Executable))
$fixture = Join-Path $OutputDirectory 'fixture'
New-Item -ItemType Directory -Force $fixture | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
function Bytes([string]$text) { return ,([Text.Encoding]::UTF8.GetBytes($text)) }
function Hash([byte[]]$bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
function Save([string]$name, [byte[]]$bytes) {
    $path = Join-Path $fixture $name
    [IO.File]::WriteAllBytes($path, $bytes)
    return $path
}
function Invoke-Driver([string]$arguments, $listener, [byte[]]$body, [string]$contentType) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Executable
    $start.Arguments = $arguments
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    if (-not $process.Start()) { throw "Cannot start driver: $arguments" }
    if ($listener) {
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not $listener.Pending()) {
            if ([DateTime]::UtcNow -ge $deadline) { throw "No HTTP request: $arguments" }
            Start-Sleep -Milliseconds 10
        }
        $client = $listener.AcceptTcpClient()
        try {
            $stream = $client.GetStream()
            $request = New-Object byte[] 4096
            $null = $stream.Read($request, 0, $request.Length)
            $header = Bytes "HTTP/1.1 200 OK`r`nContent-Type: $contentType`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n"
            $stream.Write($header, 0, $header.Length)
            $stream.Write($body, 0, $body.Length)
        } finally { $client.Close(); $listener.Stop() }
    }
    $stdout = New-Object IO.MemoryStream
    $stderr = New-Object IO.MemoryStream
    $process.StandardOutput.BaseStream.CopyTo($stdout)
    $process.StandardError.BaseStream.CopyTo($stderr)
    if (-not $process.WaitForExit(10000)) { $process.Kill(); throw "Driver timeout: $arguments" }
    $outBytes = $stdout.ToArray()
    $errBytes = $stderr.ToArray()
    return [ordered]@{
        exit = $process.ExitCode
        stdout_sha256 = Hash $outBytes
        stderr_sha256 = Hash $errBytes
        stdout = [Text.Encoding]::UTF8.GetString($outBytes)
        stderr = [Text.Encoding]::UTF8.GetString($errBytes)
    }
}

$cases = @()
function Add-FileCase([string]$id, [string]$name, [byte[]]$body) {
    $path = Save $name $body
    $run = Invoke-Driver "parse `"$path`"" $null $null $null
    $script:cases += [ordered]@{ id=$id; input_sha256=(Hash $body); result=$run }
}
function Add-HttpCase([string]$id, [string]$contentType, [byte[]]$body) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $run = Invoke-Driver "fetch-parse http://127.0.0.1:$port/document.xhtml" $listener $body $contentType
    $script:cases += [ordered]@{ id=$id; input_sha256=(Hash $body); content_type=$contentType; result=$run }
}

$utf8Bom = [Text.Encoding]::UTF8.GetPreamble()
$utf16Le = [Text.Encoding]::Unicode
$utf16Be = [Text.Encoding]::BigEndianUnicode
Add-FileCase 'bom-over-meta' 'bom-over-meta.html' ([byte[]]($utf8Bom + (Bytes ('<meta charset=windows-1252><p>' + [char]0x00e9 + '</p>'))))
Add-FileCase 'meta-windows1252' 'meta-windows1252.html' ([byte[]]((Bytes '<meta charset=windows-1252><p>') + [byte[]](0x93,0x78,0x94) + (Bytes '</p>')))
Add-FileCase 'default-windows1252' 'default-windows1252.html' ([byte[]]((Bytes '<p>') + [byte[]](0x80) + (Bytes '</p>')))
Add-FileCase 'utf16le-bom' 'utf16le-bom.html' ([byte[]]($utf16Le.GetPreamble() + $utf16Le.GetBytes(('<p>' + [char]0x96ea + '</p>'))))
Add-FileCase 'utf16be-bom' 'utf16be-bom.html' ([byte[]]($utf16Be.GetPreamble() + $utf16Be.GetBytes(('<p>' + [char]0x96ea + '</p>'))))
Add-FileCase 'malformed-utf8-replacement' 'malformed-utf8.html' ([byte[]]((Bytes '<meta charset=utf-8><p>A') + [byte[]](0xff) + (Bytes 'B</p>')))
Add-FileCase 'xml-valid-case' 'xml-valid.xhtml' (Bytes '<Root><Item/></Root>')
Add-FileCase 'xml-malformed-fatal' 'xml-malformed.xhtml' (Bytes '<Root><Item></Root>')
Add-FileCase 'xml-undecoded-byte' 'xml-undecoded.xml' ([byte[]]((Bytes '<Root>') + [byte[]](0x93) + (Bytes '</Root>')))
Add-HttpCase 'http-transport-over-meta' 'text/html; charset="windows-1252"' ([byte[]]((Bytes '<meta charset=utf-8><p>') + [byte[]](0x93) + (Bytes '</p>')))
Add-HttpCase 'http-bom-over-transport' 'text/html; charset=windows-1252' ([byte[]]($utf8Bom + (Bytes ('<p>' + [char]0x00e9 + '</p>'))))
Add-HttpCase 'http-xml-mode' 'application/xhtml+xml; charset=windows-1252' (Bytes '<Root><Item></Root>')

$projection = ConvertTo-Json -InputObject $cases -Depth 8
$report = [ordered]@{
    stage = $Stage
    driver_revision = $DriverRevision
    executable_sha256 = (Get-FileHash $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
    cases = $cases
    projection_sha256 = Hash (Bytes $projection)
}
$json = ConvertTo-Json -InputObject $report -Depth 10
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'report.json'), $json + "`n", $utf8)
Write-Output ("stage={0} cases={1} executable={2} projection={3}" -f $Stage,$cases.Count,$report.executable_sha256,$report.projection_sha256)
