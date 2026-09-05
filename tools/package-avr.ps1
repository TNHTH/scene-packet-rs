param([string]$Profile = "release")

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

Push-Location "$Root\desktop\ui"
npm run build --silent
Pop-Location

cargo build --$Profile -p avr

$Exe = Join-Path $Root "target\$Profile\avr.exe"
$SizeMb = [math]::Round((Get-Item $Exe).Length / 1MB, 2)
Write-Host "avr.exe size: $SizeMb MB"

$Dist = Join-Path $Root "dist"
$Stage = Join-Path $Dist "avr-portable"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item $Exe (Join-Path $Stage "avr.exe")
New-Item -ItemType File -Path (Join-Path $Stage "avr.portable") -Force | Out-Null

$Zip = Join-Path $Dist "avr-portable.zip"
if (Test-Path $Zip) { Remove-Item $Zip }
Compress-Archive -Path "$Stage\*" -DestinationPath $Zip
Write-Host "Created $Zip"
