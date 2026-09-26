# Windows Bildirim Merkezi'ndeki bildirimleri (wpndatabase.db) JSON olarak verir.
# ii'nin sağ panelindeki bildirim listesi için. Veritabanı kullanımda olduğu için kopyası okunur.
param([int]$Limit = 150)

[Console]::OutputEncoding = [Text.Encoding]::UTF8
$sqlite = Join-Path $env:USERPROFILE 'scoop\apps\sqlite\current\sqlite3.exe'
$src = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Notifications'
$tmp = Join-Path $env:TEMP 'll-wpn'
if (-not (Test-Path $sqlite)) { '[]'; exit }
New-Item -ItemType Directory -Force $tmp | Out-Null
foreach ($f in 'wpndatabase.db', 'wpndatabase.db-wal', 'wpndatabase.db-shm') {
    $p = Join-Path $src $f
    $t = Join-Path $tmp $f
    if (Test-Path $p) { Copy-Item $p $t -Force } elseif (Test-Path $t) { Remove-Item $t -Force }
}

$query = "SELECT n.Id AS id, n.ArrivalTime AS t, h.PrimaryId AS aumid, CAST(n.Payload AS TEXT) AS payload " +
         "FROM Notification n JOIN NotificationHandler h ON n.HandlerId = h.RecordId " +
         "WHERE n.Type = 'toast' ORDER BY n.ArrivalTime DESC LIMIT $Limit;"
$raw = & $sqlite -readonly -json (Join-Path $tmp 'wpndatabase.db') $query 2>$null
if (-not $raw) { '[]'; exit }
$rows = ($raw -join "`n") | ConvertFrom-Json

# Uygulama adlarını ve ikonlarını overview'ün uygulama listesinden bul
$apps = @()
$appsFile = Join-Path $env:LOCALAPPDATA 'LogicalLunge\state\apps.json'
if (Test-Path $appsFile) { $apps = [IO.File]::ReadAllText($appsFile) | ConvertFrom-Json }

$out = foreach ($r in $rows) {
    $title = ''; $body = ''
    try {
        $xml = [xml]$r.payload
        $texts = @($xml.SelectNodes('//text') | ForEach-Object { $_.InnerText.Trim() } | Where-Object { $_ })
        if ($texts.Count -gt 0) { $title = $texts[0] }
        if ($texts.Count -gt 1) { $body = ($texts[1..($texts.Count - 1)]) -join "`n" }
    } catch {}
    $aumid = [string]$r.aumid
    $key = ($aumid -split '[-!]')[-1]
    $app = $apps | Where-Object { $_.path -and ($_.path -like "*$aumid*" -or ($key.Length -ge 6 -and $_.path -like "*$key*")) } | Select-Object -First 1
    $name = if ($app) { $app.name } else { ($aumid -split '[_!\.]')[0] }
    [ordered]@{
        id    = [long]$r.id
        time  = [long](([long]$r.t - 116444736000000000) / 10000)  # FILETIME -> unix ms
        app   = $name
        aumid = $aumid
        icon  = if ($app) { $app.icon } else { $null }
        title = $title
        body  = $body
    }
}
ConvertTo-Json @($out) -Depth 3 -Compress
