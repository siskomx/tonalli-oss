param([switch]$Large, [switch]$ProofOnly,
    [string]$Xorriso = (Join-Path $env:LOCALAPPDATA 'Tonalli/tools/msys64/usr/bin/xorriso.exe'),
    [string]$CheckerPython = (Join-Path $env:USERPROFILE '.codex/tools/tonalli/python-pycdlib-1.22.0/Scripts/python.exe'))
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..')).TrimEnd('\', '/')
function Assert-WorkspacePath([string]$Path) {
    $absolute = [IO.Path]::GetFullPath($Path)
    if (-not $absolute.StartsWith($workspace + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Output escapes workspace: $absolute"
    }
    $part = $absolute
    while ($part.Length -ge $workspace.Length) {
        if ((Test-Path -LiteralPath $part) -and ((Get-Item -LiteralPath $part -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Output has a reparse-point component: $part"
        }
        $part = [IO.Path]::GetDirectoryName($part)
        if (-not $part) { break }
    }
    return $absolute
}
$run = Assert-WorkspacePath (Join-Path $workspace ('target/iso-fixtures-' + [Guid]::NewGuid().ToString('N')))
[void][IO.Directory]::CreateDirectory($run)
$stage = Assert-WorkspacePath (Join-Path $run 'udf-stage')
[void][IO.Directory]::CreateDirectory($stage)
$name = 'Ni' + [char]0x00f1 + 'ez_y_educaci' + [char]0x00f3 + 'n_larga_fixture.zim'
$source = Join-Path $workspace 'crates/core/tests/fixtures/articles/benign-v1.zim'
[IO.File]::Copy($source, (Join-Path $stage $name), $false)
$fixedTime = [DateTime]::SpecifyKind([DateTime]::ParseExact('20261111000000', 'yyyyMMddHHmmss', [Globalization.CultureInfo]::InvariantCulture), [DateTimeKind]::Utc)
[IO.File]::SetLastWriteTimeUtc((Join-Path $stage $name), $fixedTime)

if (-not ('TonalliIsoStream' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class TonalliIsoStream {
    public static long Save(object value, string path) {
        var stream = (IStream)value;
        var count = Marshal.AllocHGlobal(4);
        try {
            using (var output = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None)) {
                var buffer = new byte[65536];
                long total = 0;
                while (true) {
                    stream.Read(buffer, buffer.Length, count);
                    int read = Marshal.ReadInt32(count);
                    if (read == 0) break;
                    if (read < 0 || read > buffer.Length || total + read > 8 * 1024 * 1024)
                        throw new InvalidDataException("Small UDF proof exceeded its bounded output");
                    output.Write(buffer, 0, read);
                    total += read;
                }
                output.Flush(true);
                return total;
            }
        } finally { Marshal.FreeHGlobal(count); }
    }
}
'@
}
function Write-Udf([string]$Destination) {
    $destination = Assert-WorkspacePath $Destination
    $image = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
    $result = $null
    try {
        $image.FileSystemsToCreate = 4
        $image.UDFRevision = 0x102
        $image.SessionStartBlock = 0
        $image.FreeMediaBlocks = 32768
        $image.VolumeName = 'TONALLI_UDF_V1'
        $image.WorkingDirectory = $run
        $image.Root.AddTree($stage, $false)
        $image.Root.CreationTime = $fixedTime
        $image.Root.LastAccessedTime = $fixedTime
        $image.Root.LastModifiedTime = $fixedTime
        foreach ($item in $image.Root) {
            $item.CreationTime = $fixedTime
            $item.LastAccessedTime = $fixedTime
            $item.LastModifiedTime = $fixedTime
        }
        $result = $image.CreateResultImage()
        [void][TonalliIsoStream]::Save($result.ImageStream, $destination)
    } finally {
        if ($null -ne $result) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($result) }
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($image)
    }
}
$udf = Assert-WorkspacePath (Join-Path $run 'udf-v1.iso')
Write-Udf $udf
Write-Udf (Join-Path $run 'udf-repro.iso')
function File-Sha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose(); $stream.Dispose() }
}
$mounted = $null
try {
    $mounted = Mount-DiskImage -ImagePath $udf -StorageType ISO -Access ReadOnly -PassThru
    $volumes = @($mounted | Get-Volume | Where-Object { $_.DriveLetter })
    if ($volumes.Count -ne 1 -or $volumes[0].FileSystem -ne 'UDF') { throw 'Expected one native UDF volume' }
    $contained = [string]$volumes[0].DriveLetter + ':\' + $name
    if ((File-Sha256 $contained) -ne (File-Sha256 $source)) { throw 'Native UDF extraction checksum differs' }
    $proof = [ordered]@{
        writer = 'Windows IMAPI2FS'; writer_version = (Get-Item -LiteralPath "$env:WINDIR/System32/imapi2fs.dll").VersionInfo.FileVersion
        reader = 'Windows UDF read-only Mount-DiskImage'; reader_version = (Get-Item -LiteralPath "$env:WINDIR/System32/drivers/udfs.sys").VersionInfo.FileVersion
        udf_revision = '1.02'; namespace_path = '/' + $name; source_sha256 = File-Sha256 $source
        bytes = (Get-Item -LiteralPath $udf).Length; sha256 = File-Sha256 $udf
        rebuild_sha256 = File-Sha256 (Join-Path $run 'udf-repro.iso')
        payload_verified = $true; bit_reproducible = ((File-Sha256 $udf) -eq (File-Sha256 (Join-Path $run 'udf-repro.iso')))
    }
    [IO.File]::WriteAllText((Join-Path $run 'udf-proof.json'), ($proof | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
} finally {
    if ($null -ne $mounted) { Dismount-DiskImage -ImagePath $udf | Out-Null }
}
if ($ProofOnly) { Write-Output $run; exit 0 }
& $CheckerPython (Join-Path $workspace 'scripts/iso-fixtures.py') --workspace $workspace --stage $run --xorriso $Xorriso $(if ($Large) { '--large' })
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
