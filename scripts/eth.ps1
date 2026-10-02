# Ethernet kartını açar/kapatır (YÖNETİCİ gerekir). Kurulumun oluşturduğu zamanlanmış görevler
# (LogicalLunge\Ethernet-On / Ethernet-Off) bunu çağırır; sağ paneldeki kutucuk görevleri çekirdekten tetikler
# (/qs/eth-toggle), kartın durumu da çekirdekten okunur (/qs/eth).
#   eth.ps1 enable | disable
param([string]$Action = '')

# Fiziksel kablolu kartlar (sanal VPN / VirtualBox / Hamachi kartları hariç)
function Get-Eth {
    Get-NetAdapter -Physical -ErrorAction SilentlyContinue |
        Where-Object { $_.MediaType -eq '802.3' -and $_.InterfaceDescription -notmatch '(?i)virtual|vpn|tap|hamachi|radmin|tailscale|wireless|wi-?fi|802\.11' }
}

switch ($Action) {
    'enable'  { Get-Eth | Enable-NetAdapter -Confirm:$false }
    'disable' { Get-Eth | Disable-NetAdapter -Confirm:$false }
}
