param(
    [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$name = 'AgyPlus'

if ($Uninstall) {
    Remove-ItemProperty -Path $runKey -Name $name -ErrorAction SilentlyContinue
    Write-Output "Removed HKCU Run entry: $name"
    exit 0
}

$exe = Join-Path $root 'target\x86_64-pc-windows-msvc\release\agy-plus.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    $exe = Join-Path $root 'target\release\agy-plus.exe'
}
if (-not (Test-Path -LiteralPath $exe)) {
    throw 'agy-plus.exe not found. Run scripts/build-windows.ps1 first.'
}

Set-ItemProperty -Path $runKey -Name $name -Value "`"$exe`" --login-startup"
Write-Output "Installed HKCU Run entry: $name -> `"$exe`" --login-startup"
