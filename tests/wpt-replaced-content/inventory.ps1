param([string]$WptRoot, [string]$OutputDirectory = $PSScriptRoot, [int]$PerCluster = 8)
$ErrorActionPreference = 'Stop'
$revision = 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
if ((& git -C $WptRoot rev-parse HEAD).Trim() -ne $revision) { throw 'Wrong WPT revision' }
$roots = @('css/css-images','html/semantics/forms','html/semantics/embedded-content','html/canvas','svg')
$patterns = [ordered]@{
 controls = '<(?:input|button|select|textarea)\b'
 images = '<img\b|object-fit\s*:'
 svg = '<svg\b|\.svg["'']'
 canvas = '<canvas\b|getContext\s*\('
 embedded = '<(?:iframe|object|embed)\b'
 media = '<(?:audio|video)\b'
}
$rows = @()
$excluded = 0
$sha = [Security.Cryptography.SHA256]::Create()
foreach ($root in $roots) {
 foreach ($file in Get-ChildItem (Join-Path $WptRoot $root) -Filter '*.html' -Recurse -File) {
  $source = [IO.File]::ReadAllText($file.FullName)
  $match = [regex]::Match($source, '<link\b(?=[^>]*\brel\s*=\s*["'']match["''])(?=[^>]*\bhref\s*=\s*["'']([^"'']+)["''])[^>]*>', 'IgnoreCase')
  if (!$match.Success) { continue }
  $relative = $file.FullName.Substring($WptRoot.TrimEnd('\').Length + 1).Replace('\','/')
  $clusters = @($patterns.Keys | Where-Object { $source -match $patterns[$_] })
  if (!$clusters.Count) { continue }
  if ($file.FullName -match '[\\/]tools[\\/]' -or $file.Name -match 'template') { $excluded++; continue }
  $hash = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($relative))).Replace('-','').ToLowerInvariant()
  $rows += [ordered]@{test=$relative; hash=$hash; clusters=$clusters; script=($source -match '<script\b'); reference=$match.Groups[1].Value}
 }
}
$counts = [ordered]@{}
$selected = @()
foreach ($cluster in $patterns.Keys) {
 $eligible = @($rows | Where-Object { $_.clusters -contains $cluster } | Sort-Object { $_.hash })
 $counts[$cluster] = [ordered]@{html_match_candidates=$eligible.Count; scripted=@($eligible | Where-Object {$_.script}).Count; selected=[Math]::Min($PerCluster,$eligible.Count)}
 foreach ($row in ($eligible | Select-Object -First $PerCluster)) {
  $selected += [ordered]@{id=($cluster+'__'+$row.test.Replace('/','__')); test=$row.test; cluster=$cluster; scripted=$row.script}
 }
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
foreach ($cluster in $patterns.Keys) {
 $items = @($rows | Where-Object { $_.clusters -contains $cluster } | Sort-Object { $_.hash })
 $lines = @('{"revision":"'+$revision+'","cluster":"'+$cluster+'","candidates":[')
 $lines += (@($items | ForEach-Object { $_ | ConvertTo-Json -Depth 8 -Compress }) -join ",`n")
 $lines += ']}'
 $lines | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory "inventory-$cluster.json")
}
[ordered]@{schema=1;revision=$revision;selection='First N SHA256 path-ranked HTML rel=match cases per direct-source feature marker, overlaps retained';counts=$counts;excluded_tools_templates=$excluded} | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'inventory.json')
[ordered]@{schema=1;revision=$revision;selection=@{kind='sha256-path-per-marker-cluster';per_cluster=$PerCluster};sparse_paths=$roots;tests=$selected} | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'manifest.json')
$counts | ConvertTo-Json -Depth 4
