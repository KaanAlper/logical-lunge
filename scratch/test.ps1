
$ans = @("terminal", "sensors")
if ($ans -is [string] -and $ans -eq "BACK") { Write-Host "BACK" } else { Write-Host "Not back" }
$ans2 = "BACK"
if ($ans2 -is [string] -and $ans2 -eq "BACK") { Write-Host "BACK" } else { Write-Host "Not back" }

