<#
Runs scripts/package-release.ps1 through neutral junction paths so no account name or developer folder is
baked into C-compiled strings (OpenSSL, bundled C libraries, Tauri). Rust remapping cannot change those.

  ./scripts/package-release-neutral.ps1 -IdentifiersFile C:\private\identifiers.json

Junctions are created under -Root (default C:\tonalli) and removed afterwards; no real files move.
#>
[CmdletBinding()]
param(
    [string]$Root = 'C:\tonalli',
    [string]$IdentifiersFile,
    [string]$MsysRoot,
    [switch]$ReuseBuilt,
    [switch]$Ai,
    [switch]$SkipAndroid,
    [switch]$SkipWindows
)
$ErrorActionPreference = 'Stop'
if ($Root -match '\\Users\\') { throw 'Choose a -Root outside any user profile.' }
$repository = Split-Path $PSScriptRoot -Parent
$links = [ordered]@{ repo = $repository; home = $env:USERPROFILE; local = $env:LOCALAPPDATA }
$savedProfile = $env:USERPROFILE
$savedLocal = $env:LOCALAPPDATA
New-Item -ItemType Directory -Force $Root | Out-Null
foreach ($name in $links.Keys) {
    $path = Join-Path $Root $name
    if (Test-Path $path) { throw "$path already exists; remove it first." }
    New-Item -ItemType Junction -Path $path -Target $links[$name] | Out-Null
}
try {
    $env:USERPROFILE = Join-Path $Root 'home'
    $env:LOCALAPPDATA = Join-Path $Root 'local'
    Set-Location (Join-Path $Root 'repo')
    $arguments = @{}
    if ($MsysRoot) { $arguments.MsysRoot = $MsysRoot }
    if ($IdentifiersFile) { $arguments.IdentifiersFile = $IdentifiersFile }
    if ($ReuseBuilt) { $arguments.ReuseBuilt = $true }
    if ($Ai) { $arguments.Ai = $true }
    if ($SkipAndroid) { $arguments.SkipAndroid = $true }
    if ($SkipWindows) { $arguments.SkipWindows = $true }
    & (Join-Path $Root 'repo/scripts/package-release.ps1') @arguments
} finally {
    $env:USERPROFILE = $savedProfile
    $env:LOCALAPPDATA = $savedLocal
    Set-Location $PSScriptRoot
    foreach ($name in $links.Keys) { (Get-Item (Join-Path $Root $name)).Delete() }
}
