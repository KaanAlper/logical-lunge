# Logical Lunge: the next release version of an edition (semantic versioning).
# MAJOR when a commit since the edition's last release is breaking ("type!:" or
# "BREAKING CHANGE"), MINOR when one adds a feature ("feat:"), PATCH otherwise.
# -Bump patch|minor|major forces the level. Versions any tag already used
# (failed or draft attempts too) are never reused.
param([string]$Edition, [string]$BaseVersion, [string[]]$Tags, [string[]]$Commits, [string]$Bump = 'auto')

function Get-BumpLevel([string[]]$commits) {
    $level = 'patch'
    foreach ($c in $commits) {
        if (-not $c) { continue }
        if ($c -match '(?m)^\w+(\([^)]*\))?!:' -or $c -match '(?m)^BREAKING[ -]CHANGE: \S') { return 'major' }
        if ($c -match '(?m)^feat(\([^)]*\))?:') { $level = 'minor' }
    }
    $level
}

function Step([version]$v, [string]$level) {
    switch ($level) {
        'major' { [version]::new($v.Major + 1, 0, 0) }
        'minor' { [version]::new($v.Major, $v.Minor + 1, 0) }
        default { [version]::new($v.Major, $v.Minor, $v.Build + 1) }
    }
}

function Get-NextReleaseVersion([string]$base, [string]$edition, [string[]]$tags, [string[]]$commits, [string]$bump) {
    if ($edition -notin @('native-ui', 'web-ui')) { throw "Invalid edition: $edition" }
    if ($base -notmatch '^\d+\.\d+\.\d+$') { throw "Invalid VERSION: $base" }
    if ($bump -notin @('auto', 'patch', 'minor', 'major')) { throw "Invalid bump: $bump" }
    $pattern = '^v(\d+\.\d+\.\d+)-' + [regex]::Escape($edition) + '$'
    $used = @($tags | Where-Object { $_ -match $pattern } | ForEach-Object { [version]($_ -replace '^v|-.*$', '') })
    if ($used.Count -eq 0) { return ([version]$base).ToString(3) }
    $last = ($used | Sort-Object -Descending)[0]
    $level = if ($bump -eq 'auto') { Get-BumpLevel $commits } else { $bump }
    $next = Step $last $level
    if ([version]$base -gt $next) { $next = [version]$base }
    while ($used -contains $next) { $next = Step $next 'patch' }
    $next.ToString(3)
}

if ($Edition) { Get-NextReleaseVersion $BaseVersion $Edition $Tags $Commits $Bump }
