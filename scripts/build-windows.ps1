param()
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio Build Tools not found' }
    $vsPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vsPath) { throw 'MSVC x64 build tools not found' }
    & (Join-Path $vsPath 'Common7\Tools\Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
}
& npm.cmd run tauri -- build --no-bundle --target x86_64-pc-windows-msvc -- --locked
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
$version = (Get-Content -LiteralPath 'package.json' -Raw | ConvertFrom-Json).version
$releaseDir = Join-Path $PWD 'target\x86_64-pc-windows-msvc\release'
$outputDir = Join-Path $PWD 'target\portable'
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
$archive = Join-Path $outputDir "agy-plus-$version-windows-x64.zip"
Compress-Archive -LiteralPath (Join-Path $releaseDir 'agy-plus.exe') -DestinationPath $archive -Force
Write-Output "Portable package: $archive"
