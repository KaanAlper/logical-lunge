# Ethernet kutucuğu. Çıktı JSON.
#   eth.ps1 status  -> {"state":"up|disconnected|disabled|none","name":"Ethernet","desc":"...","speed":"2.5 Gbps","ip":"..."}
#   eth.ps1 toggle  -> yönetici görevini (LogicalLunge\Ethernet-On / LogicalLunge\Ethernet-Off) tetikler; izin sormaz
#   eth.ps1 enable | disable -> kartı açar/kapatır (YÖNETİCİ gerekir; zamanlanmış görev bunu çağırır)
param([string]$Action = 'status')

[Console]::OutputEncoding = [Text.Encoding]::UTF8

# Fiziksel kablolu kartlar (sanal VPN / VirtualBox / Hamachi kartları hariç)
function Get-Eth {
    Get-NetAdapter -Physical -ErrorAction SilentlyContinue |
        Where-Object { $_.MediaType -eq '802.3' -and $_.InterfaceDescription -notmatch '(?i)virtual|vpn|tap|hamachi|radmin|tailscale|wireless|wi-?fi|802\.11' }
}

switch ($Action) {
    'enable'  { Get-Eth | Enable-NetAdapter -Confirm:$false; exit }
    'disable' { Get-Eth | Disable-NetAdapter -Confirm:$false; exit }
    'toggle' {
        $a = Get-Eth | Select-Object -First 1
        if (-not $a) { '{"ok":false}'; exit }
        $task = if ($a.Status -eq 'Disabled') { 'LogicalLunge\Ethernet-On' } else { 'LogicalLunge\Ethernet-Off' }
        $null = schtasks /run /tn $task 2>&1
        '{"ok":' + ($(if ($LASTEXITCODE -eq 0) { 'true' } else { 'false' })) + ',"needSetup":' + ($(if ($LASTEXITCODE -ne 0) { 'true' } else { 'false' })) + '}'
        exit
    }
    default {
        $a = Get-Eth | Select-Object -First 1
        if (-not $a) { '{"state":"none"}'; exit }
        $state = switch ($a.Status) { 'Up' { 'up' } 'Disabled' { 'disabled' } default { 'disconnected' } }
        $ip = (Get-NetIPAddress -InterfaceIndex $a.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue | Select-Object -First 1).IPAddress
        [ordered]@{ state = $state; name = $a.Name; desc = $a.InterfaceDescription; speed = $a.LinkSpeed; ip = $ip } | ConvertTo-Json -Compress
    }
}
