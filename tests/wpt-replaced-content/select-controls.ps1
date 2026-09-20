param([string]$WptRoot, [string]$OutputDirectory=$PSScriptRoot)
$ErrorActionPreference='Stop'
$revision='c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
if ((git -C $WptRoot rev-parse HEAD).Trim() -ne $revision) {throw 'Wrong WPT pin'}
$roots=@('html/semantics/forms','html/rendering/widgets','css/css-ui','css/css-forms')
$rows=@()
$sha=[Security.Cryptography.SHA256]::Create()
foreach($root in $roots) {
 $directory=Join-Path $WptRoot $root
 if (!(Test-Path $directory)) {continue}
 foreach($file in Get-ChildItem $directory -Filter '*.html' -Recurse -File) {
  $s=[IO.File]::ReadAllText($file.FullName)
  $ref=[regex]::Match($s,'<link\b(?=[^>]*\brel\s*=\s*["'']match["''])(?=[^>]*\bhref\s*=\s*["'']([^"'']+)["''])[^>]*>','IgnoreCase')
  if (!$ref.Success -or $s -match '<script\b' -or $file.Name -match 'template') {continue}
  $kinds=@()
  foreach($tag in [regex]::Matches($s,'<input\b[^>]*>','IgnoreCase')) {
   $type=[regex]::Match($tag.Value,'\btype\s*=\s*(?:"([^"]*)"|''([^'']*)''|([^\s>]+))','IgnoreCase')
   $v='text'
   if($type.Success){$v=($type.Groups[1].Value+$type.Groups[2].Value+$type.Groups[3].Value).ToLowerInvariant()}
   switch -Regex ($v) {
    '^(text|search|email|tel|url|password|number)$' {$kinds+='text-input'}
    '^checkbox$' {$kinds+='checkbox'}
    '^radio$' {$kinds+='radio'}
    '^(button|submit|reset)$' {$kinds+='button'}
   }
  }
  foreach($kind in 'button','select','textarea'){if($s -match "<$kind\b"){$kinds+=$kind}}
  $kinds=@($kinds|Sort-Object -Unique)
  if(!$kinds.Count){continue}
  $path=$file.FullName.Substring($WptRoot.TrimEnd('\').Length+1).Replace('\','/')
  $hash=[BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($path))).Replace('-','').ToLowerInvariant()
  $rows += [ordered]@{test=$path;hash=$hash;kinds=$kinds;reference=$ref.Groups[1].Value}
 }
}
$selected=@();$counts=[ordered]@{};$used=@{}
foreach($kind in 'text-input','checkbox','radio','button','select','textarea') {
 $eligible=@($rows|Where-Object {$_.kinds -contains $kind}|Sort-Object {$_.hash})
 $counts[$kind]=$eligible.Count
 foreach($row in $eligible|Where-Object {!$used.ContainsKey($_.test)}|Select-Object -First 4) {
  $used[$row.test]=$true
  $selected += [ordered]@{id=($kind+'__'+$row.test.Replace('/','__'));test=$row.test;kind=$kind}
 }
}
New-Item -ItemType Directory -Force $OutputDirectory|Out-Null
[ordered]@{revision=$revision;selection='First four unused SHA256-path-ranked static HTML rel=match cases per control marker, fixed kind order; no test/reference rewriting';candidate_counts=$counts;candidates=$rows} |ConvertTo-Json -Depth 8|Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'controls-inventory.json')
[ordered]@{schema=1;revision=$revision;selection=@{kind='sha256-static-control-kinds-unique';per_kind=4};sparse_paths=$roots;tests=$selected}|ConvertTo-Json -Depth 8|Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'controls-manifest.json')
$counts|ConvertTo-Json
$selected|ForEach-Object {$_.id}
