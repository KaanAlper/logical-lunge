# Kısayol düzenleyicisi için pencere yöneticisi kısayolları (config.yaml > keybindings). Çıktı JSON.
#   keybinds-gwm.ps1 list                 -> [{"index":0,"commands":[...],"bindings":["Super+F", ...]}]
#   keybinds-gwm.ps1 set <index> <combo>  -> ilk kısayolu değiştirir (diğerleri kalır), config'i yeniden yükler
#   keybinds-gwm.ps1 reset                -> ilk düzenlemeden önce saklanan özgün kısayollara döner
param([string]$Action = 'list', [int]$Index = -1, [string]$Combo = '')

[Console]::OutputEncoding = [Text.Encoding]::UTF8
$cfg = Join-Path $env:USERPROFILE '.config\logical-lunge\config.yaml'
$state = Join-Path $env:LOCALAPPDATA 'LogicalLunge\state'
$orig = Join-Path $state 'tiling-keybindings.default.json'
New-Item -ItemType Directory -Force $state | Out-Null

# Pencere yöneticisinin "lwin+shift+f" <-> bizim "Super+Shift+F"
$toUi = @{ lwin = 'Super'; rwin = 'Super'; ctrl = 'Ctrl'; control = 'Ctrl'; shift = 'Shift'; alt = 'Alt'; menu = 'Alt'
    left = 'Left'; right = 'Right'; up = 'Up'; down = 'Down'; enter = 'Enter'; space = 'Space'; tab = 'Tab'
    page_up = 'PageUp'; page_down = 'PageDown'; oem_1 = ';'; oem_7 = "'"; oem_comma = 'Comma'; oem_period = 'Period'; escape = 'Escape' }
function To-Ui([string]$b) {
    ($b -split '\+' | ForEach-Object { if ($toUi.ContainsKey($_)) { $toUi[$_] } elseif ($_.Length -eq 1) { $_.ToUpper() } else { $_.Substring(0,1).ToUpper() + $_.Substring(1) } }) -join '+'
}
function To-Gwm([string]$c) {
    ($c -split '\+' | ForEach-Object {
        switch ($_) { 'Super' { 'lwin' } 'Ctrl' { 'ctrl' } 'Shift' { 'shift' } 'Alt' { 'alt' } 'PageUp' { 'page_up' } 'PageDown' { 'page_down' }
            ';' { 'oem_1' } "'" { 'oem_7' } 'Comma' { 'oem_comma' } 'Period' { 'oem_period' } default { $_.ToLower() } }
    }) -join '+'
}

function Read-Entries([string[]]$lines) {
    $inKb = $false; $list = @(); $pending = $null
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $l = $lines[$i]
        if ($l -match '^keybindings:') { $inKb = $true; continue }
        if ($inKb -and $l -match '^\S') { break }
        if (-not $inKb) { continue }
        if ($l -match "^\s*- commands:\s*\[(.*)\]\s*$") { $pending = [ordered]@{ commands = @([regex]::Matches($Matches[1], "'([^']*)'") | ForEach-Object { $_.Groups[1].Value }) } }
        elseif ($pending -and $l -match "^\s*bindings:\s*\[(.*)\]\s*$") {
            $pending.bindings = @([regex]::Matches($Matches[1], "'([^']*)'") | ForEach-Object { $_.Groups[1].Value })
            $pending.line = $i
            $list += , $pending; $pending = $null
        }
    }
    $list
}

$lines = [IO.File]::ReadAllLines($cfg)
$entries = Read-Entries $lines
switch ($Action) {
    'list' {
        $out = for ($i = 0; $i -lt $entries.Count; $i++) {
            [ordered]@{ index = $i; commands = $entries[$i].commands; bindings = @($entries[$i].bindings | ForEach-Object { To-Ui $_ }) }
        }
        ConvertTo-Json -InputObject @($out) -Depth 4 -Compress
    }
    'set' {
        if ($Index -lt 0 -or $Index -ge $entries.Count) { '{"ok":false,"error":"index"}'; exit }
        if (-not (Test-Path $orig)) {
            # Özgün kısayolları bir kez sakla (sıfırlama için)
            [IO.File]::WriteAllText($orig, (ConvertTo-Json -InputObject @($entries | ForEach-Object { , @($_.bindings) }) -Depth 3 -Compress), (New-Object Text.UTF8Encoding $false))
        }
        $e = $entries[$Index]
        $b = @($e.bindings)
        if ($Combo -eq '') { $b = @($b | Select-Object -Skip 1) } else { $new = To-Gwm $Combo; if ($b.Count -gt 0) { $b[0] = $new } else { $b = @($new) } }
        $indent = ($lines[$e.line] -replace '^(\s*).*', '$1')
        $lines[$e.line] = $indent + 'bindings: [' + (($b | ForEach-Object { "'$_'" }) -join ', ') + ']'
        [IO.File]::WriteAllLines($cfg, $lines, (New-Object Text.UTF8Encoding $false))
        '{"ok":true}'
    }
    'reset' {
        if (-not (Test-Path $orig)) { '{"ok":true}'; exit }
        $saved = Get-Content $orig -Raw | ConvertFrom-Json
        for ($i = 0; $i -lt $entries.Count -and $i -lt $saved.Count; $i++) {
            $e = $entries[$i]; $indent = ($lines[$e.line] -replace '^(\s*).*', '$1')
            $lines[$e.line] = $indent + 'bindings: [' + ((@($saved[$i]) | ForEach-Object { "'$_'" }) -join ', ') + ']'
        }
        [IO.File]::WriteAllLines($cfg, $lines, (New-Object Text.UTF8Encoding $false))
        Remove-Item $orig -Force
        '{"ok":true}'
    }
}
if ($Action -ne 'list') {
    # Pencere yöneticisine yeniden yüklet
    try {
        $ws = New-Object System.Net.WebSockets.ClientWebSocket
        $ws.ConnectAsync([Uri]'ws://127.0.0.1:6123', [Threading.CancellationToken]::None).Wait(2000) | Out-Null
        $m = [Text.Encoding]::UTF8.GetBytes('command wm-reload-config')
        $ws.SendAsync((New-Object ArraySegment[byte] -ArgumentList (, $m)), 'Text', $true, [Threading.CancellationToken]::None).Wait(2000) | Out-Null
        Start-Sleep -Milliseconds 200; $ws.Dispose()
    } catch {}
}
