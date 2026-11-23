<#
Writes a ZIP whose entry names always use "/" between folders, with the folder itself as the top
level (like Compress-Archive -Path <folder>). Windows PowerShell's Compress-Archive writes "\", which
other tools, and Tonalli's own updater, should not have to guess at.

  scripts/new-portable-zip.ps1 -Folder C:\build\Windows-x64 -Zip C:\build\Tonalli-Windows-x64.zip
#>
param(
    [Parameter(Mandatory = $true)][string]$Folder,
    [Parameter(Mandatory = $true)][string]$Zip
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = (Get-Item -LiteralPath $Folder).FullName.TrimEnd('\', '/')
$parent = Split-Path -Parent $root
$stream = [System.IO.File]::Open($Zip, [System.IO.FileMode]::CreateNew)
try {
    $archive = New-Object System.IO.Compression.ZipArchive($stream, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        Get-ChildItem -LiteralPath $root -Recurse -File | ForEach-Object {
            $name = $_.FullName.Substring($parent.Length).TrimStart('\', '/').Replace('\', '/')
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $_.FullName, $name, [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    } finally { $archive.Dispose() }
} finally { $stream.Dispose() }
