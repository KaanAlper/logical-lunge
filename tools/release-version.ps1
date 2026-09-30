param([string]$Edition, [string]$BaseVersion, [string[]]$Tags)

function Get-NextReleaseVersion([string]$base, [string]$edition, [string[]]$tags) {
    if ($edition -notin @('native-ui', 'web-ui')) { throw "Invalid edition: $edition" }
    if ($base -notmatch '^\d+\.\d+\.\d+$') { throw "Invalid VERSION: $base" }
    $next = [version]$base
    $pattern = '^v(\d+\.\d+\.\d+)-' + [regex]::Escape($edition) + '$'
    foreach ($tag in $tags) {
        if ($tag -match $pattern) {
            $v = [version]$Matches[1]
            if ($v -ge $next) { $next = [version]::new($v.Major, $v.Minor, $v.Build + 1) }
        }
    }
    $next.ToString(3)
}

if ($Edition) { Get-NextReleaseVersion $BaseVersion $Edition $Tags }
