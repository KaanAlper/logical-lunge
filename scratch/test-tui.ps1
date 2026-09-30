
$E = [char]27
function Fg([string]$hex) { $h = $hex.TrimStart("#"); "$E[38;2;$([Convert]::ToInt32($h.Substring(0, 2), 16));$([Convert]::ToInt32($h.Substring(2, 2), 16));$([Convert]::ToInt32($h.Substring(4, 2), 16))m" }
$R = "$E[0m"
function Paint([string]$hex, [string]$s) { (Fg $hex) + $s + $R }

function Show-Menu {
    param([string]$Header, [object[]]$Items, [string]$Default, [bool]$Multi, [bool]$IsColor)
    $sel = 0
    for ($i=0; $i -lt $Items.Count; $i++) { if ($Items[$i][1] -eq $Default) { $sel = $i } }
    $selected = @()
    if ($Multi -and $Default) { $selected = @($Default) } # just an example for testing

    $drawn = 0
    while ($true) {
        # clear previous lines
        if ($drawn -gt 0) { Write-Host -NoNewline "$E[$($drawn)A$E[J" }
        $out = ""
        $out += "  " + (Paint "#b69df8" $Header) + "`n"
        
        $accent = if ($IsColor) { $Items[$sel][1] } else { "#b69df8" }
        if ($IsColor -and $Items[$sel][1] -eq "custom") { $accent = "#b69df8" }

        for ($i=0; $i -lt $Items.Count; $i++) {
            $isSel = ($i -eq $sel)
            $prefix = if ($Multi) { if ($selected -contains $Items[$i][1]) { "? " } else { "? " } } else { "  " }
            $cur = if ($isSel) { (Paint $accent "? ") } else { "  " }
            $text = if ($isSel) { Paint $accent $Items[$i][0] } else { $Items[$i][0] }
            
            if ($IsColor -and $Items[$i][1] -ne "custom") {
                $text = (Paint $Items[$i][1] "--") + " " + $text
            }
            $out += "  " + $cur + $prefix + $text + "`n"
        }
        Write-Host -NoNewline $out
        $drawn = $Items.Count + 1

        $k = [Console]::ReadKey($true)
        if ($k.Key -eq "UpArrow") { $sel = ($sel - 1 + $Items.Count) % $Items.Count }
        elseif ($k.Key -eq "DownArrow") { $sel = ($sel + 1) % $Items.Count }
        elseif ($k.Key -eq "LeftArrow") { Write-Host -NoNewline "$E[$($drawn)A$E[J"; return "BACK" }
        elseif ($k.Key -eq "Spacebar" -and $Multi) {
            $val = $Items[$sel][1]
            if ($selected -contains $val) { $selected = @($selected | Where-Object { $_ -ne $val }) }
            else { $selected += $val }
        }
        elseif ($k.Key -eq "Enter") {
            Write-Host -NoNewline "$E[$($drawn)A$E[J"
            if ($Multi) { return $selected } else { return $Items[$sel][1] }
        }
        elseif ($k.Key -eq "C" -and ($k.Modifiers -band [ConsoleModifiers]::Control)) {
            Write-Host "`nCancelled"
            exit 1
        }
    }
}

