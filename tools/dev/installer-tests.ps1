# Pure installer logic only: no downloads, process stops, registry writes or UAC.
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
function Import-Function([string]$path, [string]$name) {
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw ($errors | Out-String) }
    $fn = $ast.Find({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name }, $true)
    if (-not $fn) { throw "Missing function: $name" }
    . ([scriptblock]::Create($fn.Extent.Text.Replace("function $name", "function global:$name")))
}
function Assert($ok, [string]$message) { if (-not $ok) { throw $message } }
Import-Function (Join-Path $root 'install.ps1') 'Select-EditionRelease'
Import-Function (Join-Path $root 'install.ps1') 'Is-Confirmed'
Import-Function (Join-Path $root 'tools\release-version.ps1') 'Get-NextReleaseVersion'
Import-Function (Join-Path $root 'installer\setup.ps1') 'Repair-LegacyWindowRules'
function Release([string]$tag, [string]$edition, [string]$version, [bool]$draft = $false, [bool]$pre = $false, [bool]$sha = $true) {
    $name = "LogicalLunge-$edition-$version.zip"
    $assets = @([pscustomobject]@{ name = $name; browser_download_url = "https://example.invalid/$name"; size = 10 })
    if ($sha) { $assets += [pscustomobject]@{ name = "$name.sha256"; browser_download_url = "https://example.invalid/$name.sha256" } }
    [pscustomobject]@{ tag_name = $tag; draft = $draft; prerelease = $pre; assets = $assets }
}
$releases = @(
    (Release 'v9.0.0-web-ui' 'web-ui' '9.0.0'),
    (Release 'v0.2.9-native-bar' 'native-bar' '0.2.9'),
    (Release 'v0.2.10-native-bar' 'native-bar' '0.2.10'),
    (Release 'v0.2.11-native-bar' 'native-bar' '0.2.11' $true),
    (Release 'v0.2.12-native-bar' 'native-bar' '0.2.12' $false $true),
    (Release 'v0.2.13-native-bar' 'web-ui' '0.2.13'),
    (Release 'v0.2.14-native-bar' 'native-bar' '0.2.14' $false $false $false)
)
Assert ((Select-EditionRelease $releases 'native-bar').tag_name -eq 'v0.2.10-native-bar') 'native selection crossed editions, picked a draft/prerelease, incomplete asset or lexicographic version'
Assert ((Select-EditionRelease $releases 'web-ui').tag_name -eq 'v9.0.0-web-ui') 'web selection failed'
Assert ($null -eq (Select-EditionRelease @() 'native-bar')) 'empty release list must not fall back to global latest'
Assert (-not (Is-Confirmed 'BACK')) 'BACK must never confirm cancellation'
Assert (-not (Is-Confirmed $false)) 'No must not confirm'
Assert (Is-Confirmed $true) 'Yes must confirm'
$tags = @('v0.2.10-native-bar', 'v9.0.0-web-ui', 'v0.2.9-native-bar')
Assert ((Get-NextReleaseVersion '0.2.0' 'native-bar' $tags) -eq '0.2.11') 'native version is not independent'
Assert ((Get-NextReleaseVersion '0.3.0' 'native-bar' $tags) -eq '0.3.0') 'base version bump was lost'
Assert ((Get-NextReleaseVersion '0.2.0' 'web-ui' @()) -eq '0.2.0') 'first release version is wrong'
$legacy = "      - window_title: { regex: '^(Open|Save|Save As|Aç|Kaydet|Farklı Kaydet).*' }`r`n      - window_class: { equals: '#32770' }`r`n      - window_title: { equals: 'My dialog' }`r`n    follow_native_border: true`r`n"
$fixed = Repair-LegacyWindowRules $legacy
Assert (-not $fixed.Contains('Open|Save')) 'legacy title rule survived migration'
Assert ($fixed.Contains("equals: '#32770'") -and $fixed.Contains("equals: 'My dialog'")) 'dialog or custom rules were removed'
Assert ($fixed.Contains('follow_native_border: false')) 'fullscreen outline migration failed'
Assert ((Repair-LegacyWindowRules $fixed) -eq $fixed) 'migration must be idempotent'
'PASS: 13 installer/release regression checks'
