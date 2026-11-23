[CmdletBinding()]
param([string]$Apk, [switch]$Ai, [switch]$Mesh)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrEmpty($Apk)) { $Apk = Join-Path $PSScriptRoot 'app/build/outputs/apk/debug/app-debug.apk' }
$generatedRoot = if ($Ai) { "$PSScriptRoot/app/build/generated/ai" } else { "$PSScriptRoot/app/build/generated" }
if ($Mesh) { $generatedRoot = if ($Ai) { "$PSScriptRoot/app/build/generated/ai-mesh" } else { "$PSScriptRoot/app/build/generated/mesh" } }
if (-not (Test-Path -LiteralPath $Apk)) { throw "Build the APK first: $Apk" }
$apkPath = (Resolve-Path -LiteralPath $Apk).Path
$sdk = if ($env:TONALLI_CI_TOOLS -eq '1') { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android/Sdk' }
$aapt = Join-Path $sdk 'build-tools/35.0.0/aapt.exe'
$badging = & $aapt dump badging $apkPath
if ($LASTEXITCODE -ne 0) { throw 'aapt failed' }
if (-not ($badging -match "package: name='org.tonalli.spike'")) { throw 'Unexpected package' }
if (-not ($badging -match "sdkVersion:'34'")) { throw 'Unexpected minimum SDK' }
if (-not ($badging -match "targetSdkVersion:'34'")) { throw 'Unexpected target SDK' }
$permissions = @($badging | Where-Object { $_ -match '^uses-permission:' })
$expectedPermissions = @('INTERNET', 'RUN_USER_INITIATED_JOBS', 'ACCESS_NETWORK_STATE', 'ACCESS_WIFI_STATE', 'CHANGE_WIFI_STATE',
    'ACCESS_COARSE_LOCATION', 'ACCESS_FINE_LOCATION',
    'CHANGE_WIFI_MULTICAST_STATE', 'WAKE_LOCK', 'FOREGROUND_SERVICE', 'FOREGROUND_SERVICE_DATA_SYNC', 'FOREGROUND_SERVICE_MEDIA_PLAYBACK', 'FOREGROUND_SERVICE_SPECIAL_USE',
    'POST_NOTIFICATIONS', 'NEARBY_WIFI_DEVICES', 'BLUETOOTH_SCAN', 'BLUETOOTH_CONNECT',
    'FOREGROUND_SERVICE_CONNECTED_DEVICE', 'REQUEST_INSTALL_PACKAGES') | ForEach-Object { "android.permission.$_" }
$actualPermissions = @($permissions | ForEach-Object {
    if ($_ -match "name='([^']+)'") { $Matches[1] }
})
if (@(Compare-Object $expectedPermissions $actualPermissions).Count -ne 0) { throw 'Unexpected LAN shell permission inventory' }
$manifestTree = (& $aapt dump xmltree $apkPath AndroidManifest.xml) -join "`n"
if ($LASTEXITCODE -ne 0 -or $manifestTree -notmatch 'android:networkSecurityConfig') { throw 'Missing network security configuration' }
$activityBlocks = [regex]::Matches($manifestTree, '(?ms)E: activity .*?(?=\n\s*E: activity |\z)')
foreach ($class in @('org.tonalli.spike.web.WebProbeActivity', 'org.tonalli.spike.notes.NotesProbeActivity', 'org.tonalli.spike.benchmark.BenchmarkActivity', 'org.tonalli.spike.saf.RemovableProbeActivity', 'org.tonalli.spike.lan.LanProbeActivity', 'org.tonalli.spike.ai.AiProbeActivity')) {
    $activities = @($activityBlocks | Where-Object { $_.Value -match [regex]::Escape($class) })
    if ($activities.Count -ne 1 -or $activities[0].Value -notmatch 'android:exported[^\n]*0x0' -or
        $activities[0].Value -notmatch 'android:launchMode[^\n]*0x2') { throw "Activity must be private and singleTask: $class" }
}
foreach ($class in @('org.tonalli.spike.reader.OpenContentActivity', 'org.tonalli.spike.reader.ReaderIndexActivity', 'org.tonalli.spike.media.MediaImportActivity')) {
    $picker = @($activityBlocks | Where-Object { $_.Value -match [regex]::Escape($class) })
    if ($picker.Count -ne 1 -or $picker[0].Value -notmatch 'android:exported[^\n]*0x0') {
        throw "Reader picker must be private: $class"
    }
}
$indexEnglish = [xml](Get-Content -LiteralPath "$PSScriptRoot/app/src/main/res/values/index_strings.xml" -Raw)
$indexSpanish = [xml](Get-Content -LiteralPath "$PSScriptRoot/app/src/main/res/values-es/index_strings.xml" -Raw)
$englishKeys = @($indexEnglish.resources.string | ForEach-Object { $_.name })
$spanishKeys = @($indexSpanish.resources.string | ForEach-Object { $_.name })
if ($englishKeys.Count -eq 0 -or @($englishKeys | Select-Object -Unique).Count -ne $englishKeys.Count -or
    @($spanishKeys | Select-Object -Unique).Count -ne $spanishKeys.Count -or
    @(Compare-Object $englishKeys $spanishKeys).Count -ne 0 -or
    @(@($indexEnglish.resources.string) + @($indexSpanish.resources.string) | Where-Object { [string]::IsNullOrWhiteSpace($_.InnerText) }).Count -ne 0) {
    throw 'Index setup native EN/ES resource keys or values differ'
}
$providers = @([regex]::Matches($manifestTree, '(?ms)E: provider .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)'))
if ($providers.Count -ne 1 -or $providers[0].Value -notmatch 'org\.tonalli\.spike\.update\.UpdateProvider' -or
    $providers[0].Value -notmatch 'android:exported[^\n]*0x0' -or $providers[0].Value -notmatch 'android:grantUriPermissions[^\n]*0xffffffff') {
    throw 'Expected one private update provider that grants URI access only through an Intent'
}
$services = @([regex]::Matches($manifestTree, '(?ms)E: service .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)') |
    Where-Object { $_.Value -match 'org\.tonalli\.spike\.lan\.LanService' })
if ($services.Count -ne 1 -or $services[0].Value -notmatch 'android:exported[^\n]*0x0' -or
    $services[0].Value -notmatch 'android:foregroundServiceType[^\n]*0x40000000' -or
    $services[0].Value -notmatch 'android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE') { throw 'Expected private specialUse LAN service with declared justification' }
$tvServices = @([regex]::Matches($manifestTree, '(?ms)E: service .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)') |
    Where-Object { $_.Value -match 'org\.tonalli\.spike\.tv\.TvService' })
if ($tvServices.Count -ne 1 -or $tvServices[0].Value -notmatch 'android:exported[^\n]*0x0' -or
    $tvServices[0].Value -notmatch 'android:foregroundServiceType[^\n]*0x40000000' -or
    $tvServices[0].Value -notmatch 'android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE') { throw 'Expected private specialUse TV service with declared justification' }
$downloads = @([regex]::Matches($manifestTree, '(?ms)E: service .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)') |
    Where-Object { $_.Value -match 'org\.tonalli\.spike\.downloads\.ContentDownloadService' })
if ($downloads.Count -ne 1 -or $downloads[0].Value -notmatch 'android:exported[^\n]*0x0' -or
    $downloads[0].Value -notmatch 'android.permission.BIND_JOB_SERVICE' -or
    $downloads[0].Value -notmatch '(?m)^\s*A:\s+android:process\(0x[0-9a-f]+\)=":downloads"(?:\s|$)') { throw 'Expected private signature-protected downloads process JobService' }
$music = @([regex]::Matches($manifestTree, '(?ms)E: service .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)') |
    Where-Object { $_.Value -match 'org\.tonalli\.spike\.music\.MusicService' })
if ($music.Count -ne 1 -or $music[0].Value -notmatch 'android:exported[^\n]*0x0' -or
    $music[0].Value -notmatch 'android:foregroundServiceType[^\n]*0x2(?:\s|$)') { throw 'Expected private mediaPlayback Music service' }
$copies = @([regex]::Matches($manifestTree, '(?ms)E: service .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)') |
    Where-Object { $_.Value -match 'org\.tonalli\.spike\.downloads\.NearbyCopyService' })
if ($copies.Count -ne 1 -or $copies[0].Value -notmatch 'android:exported[^\n]*0x0' -or
    $copies[0].Value -notmatch 'android:foregroundServiceType[^\n]*0x1(?:\s|$)' -or
    $copies[0].Value -match 'android:process') { throw 'Expected private in-process dataSync nearby-copy service' }
$radios = @([regex]::Matches($manifestTree, '(?ms)E: service .*?(?=\n\s*E: (?:activity|service|receiver|provider) |\z)') |
    Where-Object { $_.Value -match 'org\.tonalli\.spike\.mesh\.RadioService' })
if ($radios.Count -ne 1 -or $radios[0].Value -notmatch 'android:exported[^\n]*0x0' -or
    $radios[0].Value -notmatch 'android:foregroundServiceType[^\n]*0x10(?:\s|$)' -or
    $radios[0].Value -match 'android:process') { throw 'Expected private in-process connectedDevice radio service' }
if ($manifestTree -notmatch 'android:usesPermissionFlags[^\n]*0x10000') { throw 'Nearby Wi-Fi must declare neverForLocation' }
$networkTree = (& $aapt dump xmltree $apkPath res/xml/network_security_config.xml) -join "`n"
if ($LASTEXITCODE -ne 0 -or [regex]::Matches($networkTree, 'E: domain \(').Count -ne 1 -or
    $networkTree -notmatch '(?m)^\s*C:\s*"127\.0\.0\.1"\s*$' -or $networkTree -notmatch 'includeSubdomains[^\n]*0x0' -or
    [regex]::Matches($networkTree, 'cleartextTrafficPermitted[^\n]*0xffffffff').Count -ne 1 -or
    [regex]::Matches($networkTree, 'cleartextTrafficPermitted[^\n]*0x0').Count -ne 1) { throw 'Expected only the exact loopback cleartext exception' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Web.Extensions
$archive = [IO.Compression.ZipFile]::OpenRead($apkPath)
$inspection = Join-Path ([IO.Path]::GetTempPath()) ("tonalli-apk-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $inspection | Out-Null
try {
    $zipNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in $archive.Entries) { if (-not $zipNames.Add($entry.FullName)) { throw 'Duplicate APK ZIP entry' } }
    # Incremental packaging can retain old multidex entries and silently load stale classes.
    $dexdump = Join-Path $sdk 'build-tools/35.0.0/dexdump.exe'
    $dexClasses = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in @($archive.Entries | Where-Object { $_.FullName -match '^classes(?:[2-9]|[1-9][0-9]+)?\.dex$' })) {
        $dex = Join-Path $inspection $entry.FullName
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dex)
        $definitions = @(& $dexdump $dex | Select-String "Class descriptor\s*:\s*'([^']+)'")
        if ($LASTEXITCODE -ne 0 -or $definitions.Count -eq 0) { throw "Unable to inspect DEX definitions: $($entry.FullName)" }
        foreach ($definition in $definitions) {
            $descriptor = $definition.Matches[0].Groups[1].Value
            if (-not $dexClasses.Add($descriptor)) { throw "Duplicate APK class: $descriptor" }
        }
    }
    if ($dexClasses.Count -eq 0) { throw 'Missing APK class definitions' }
    Write-Output "DEX: $($dexClasses.Count) unique class definitions"
    $libraries = @($archive.Entries.FullName | Where-Object { $_ -like 'lib/*' })
    if ($libraries -notcontains 'lib/arm64-v8a/libtonalli_core.so') { throw 'Missing Rust ARM64 library' }
    if ($libraries -notcontains 'lib/arm64-v8a/libtonalli_maps.so') { throw 'Missing ARM64 country extractor' }
    if ($libraries -notcontains 'lib/arm64-v8a/libjnidispatch.so') { throw 'Missing JNA ARM64 library' }
    if ($libraries | Where-Object { $_ -notlike 'lib/arm64-v8a/*' }) { throw 'Non-release ABI packaged' }
    $readelf = Join-Path $sdk 'ndk/29.0.14206865/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-readelf.exe'
    foreach ($name in @('libtonalli_core.so', 'libtonalli_maps.so', 'libjnidispatch.so')) {
        $extracted = Join-Path $inspection $name
        [IO.Compression.ZipFileExtensions]::ExtractToFile($archive.GetEntry("lib/arm64-v8a/$name"), $extracted)
        $headers = & $readelf -W -h -l -d -S $extracted
        if ($LASTEXITCODE -ne 0 -or -not ($headers -match 'Machine:\s+AArch64') -or -not ($headers -match 'Class:\s+ELF64')) { throw "Invalid ARM64 ELF: $name" }
        if ($name -eq 'libtonalli_core.so' -and $headers -match '\.symtab') { throw 'Packaged release core is not stripped' }
        $loads = @($headers | Where-Object { $_ -match '^\s*LOAD\s' })
        if ($loads.Count -eq 0) { throw "No ELF LOAD segments: $name" }
        foreach ($load in $loads) {
            $fields = $load.Trim() -split '\s+'
            $alignment = [Convert]::ToInt64($fields[-1], 16)
            $offset = [Convert]::ToInt64($fields[1], 16)
            $address = [Convert]::ToInt64($fields[2], 16)
            if ($alignment -lt 16384 -or $offset % 16384 -ne $address % 16384) { throw "ELF is not 16 KiB compatible: $name" }
        }
        foreach ($line in @($headers | Where-Object { $_ -match '\(NEEDED\)' })) {
            if ($line -notmatch '\[(libc|libdl|libm|liblog)\.so\]') { throw "Unexpected native dependency: $line" }
        }
        if ($name -eq 'libtonalli_core.so') {
            $hash = [Security.Cryptography.SHA256]::Create()
            $packagedStream = [IO.File]::OpenRead($extracted)
            $stagedStream = [IO.File]::OpenRead("$generatedRoot/jniLibs/arm64-v8a/libtonalli_core.so")
            try {
                $actual = [BitConverter]::ToString($hash.ComputeHash($packagedStream)).Replace('-', '').ToLowerInvariant()
                $expected = [BitConverter]::ToString($hash.ComputeHash($stagedStream)).Replace('-', '').ToLowerInvariant()
                if ($actual -ne $expected) { throw 'Packaged Rust library differs from staged release' }
                Write-Output "Packaged Rust SHA256 $actual (matches staged release)"
            } finally { $packagedStream.Dispose(); $stagedStream.Dispose(); $hash.Dispose() }
            $bindings = [IO.File]::ReadAllText("$generatedRoot/uniffi/uniffi/tonalli_core/tonalli_core.kt")
            $functions = [regex]::Matches($bindings, 'external fun (\w+)\(')
            $nm = Join-Path (Split-Path $readelf -Parent) 'llvm-nm.exe'
            $symbols = & $nm --dynamic --defined-only $extracted
            if ($LASTEXITCODE -ne 0 -or $functions.Count -eq 0) { throw 'Unable to check UniFFI exports' }
            $exports = @($symbols | ForEach-Object { ($_.Trim() -split '\s+')[-1] })
            if ($Mesh -and $exports -notcontains 'Java_org_tonalli_spike_mesh_MeshNative_initialize') { throw 'Missing Android BLE initializer' }
            foreach ($function in $functions) {
                if ($exports -notcontains $function.Groups[1].Value) { throw "Missing generated binding export: $($function.Groups[1].Value)" }
            }
            Write-Output "UniFFI: all $($functions.Count) generated native declarations exist in packaged core"
        }
        Write-Output "$name : ARM64, 16 KiB LOAD alignment, system libraries only"
    }
    $notice = $archive.GetEntry('assets/licenses/LICENSES.md')
    if ($null -eq $notice) { throw 'Missing packaged license inventory' }
    $noticeStream = $notice.Open()
    $noticeHash = [Security.Cryptography.SHA256]::Create()
    try {
        $packaged = [Convert]::ToBase64String($noticeHash.ComputeHash($noticeStream))
        $current = [Convert]::ToBase64String($noticeHash.ComputeHash([IO.File]::ReadAllBytes("$PSScriptRoot/../LICENSES.md")))
        if ($packaged -ne $current) { throw 'Packaged LICENSES.md is stale; rebuild the APK' }
    } finally { $noticeStream.Dispose(); $noticeHash.Dispose() }
    $trustedManifests = @(Get-ChildItem -LiteralPath "$PSScriptRoot/app/src/main/assets" -Filter 'benchmark-*.json' -File)
    $trustedManifests += Get-Item -LiteralPath "$PSScriptRoot/app/src/main/assets/index-probe-manifest.json"
    $trustedManifests += Get-Item -LiteralPath "$PSScriptRoot/app/src/main/assets/index-probe-small.json"
    $trustedManifests += @(Get-ChildItem -LiteralPath "$PSScriptRoot/app/src/main/assets" -Filter 'index-probe-full.json' -File)
    foreach ($source in $trustedManifests) {
        $entry = $archive.GetEntry("assets/$($source.Name)")
        $manifestLimit = if ($source.Name -match '^index-probe-(small|full)\.json$') { 1MB } else { 65536 }
        if ($null -eq $entry -or $entry.Length -gt $manifestLimit -or $entry.Length -ne $source.Length) { throw 'Missing or oversized trusted fixture manifest' }
        $stream = $entry.Open()
        $hash = [Security.Cryptography.SHA256]::Create()
        try {
            $actual = [Convert]::ToBase64String($hash.ComputeHash($stream))
            $expected = [Convert]::ToBase64String($hash.ComputeHash([IO.File]::ReadAllBytes($source.FullName)))
            if ($actual -ne $expected) { throw 'Packaged fixture manifest differs from trusted input' }
        } finally { $stream.Dispose(); $hash.Dispose() }
    }
    $uiManifest = $archive.GetEntry('assets/ui-manifest.json')
    if ($null -eq $uiManifest -or $uiManifest.Length -gt 512 * 1024) { throw 'Missing or oversized UI manifest' }
    $uiStream = $uiManifest.Open()
    $buffer = New-Object byte[] (512 * 1024 + 1)
    $count = 0
    try {
        while ($count -lt $buffer.Length) {
            $read = $uiStream.Read($buffer, $count, $buffer.Length - $count)
            if ($read -eq 0) { break }
            $count += $read
        }
    } finally { $uiStream.Dispose() }
    if ($count -gt 512 * 1024) { throw 'Oversized UI manifest body' }
    $text = [Text.Encoding]::UTF8.GetString($buffer, 0, $count)
    $serializer = [System.Web.Script.Serialization.JavaScriptSerializer]::new()
    $serializer.MaxJsonLength = 512 * 1024
    $ui = $serializer.DeserializeObject($text)
    if ($ui['version'] -ne 1 -or $ui['id'] -notmatch '^[a-f0-9]{64}$' -or $ui['files'].Count -notin 1..2000) { throw 'Invalid UI manifest schema' }
    $uiNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $total = 0L
    foreach ($record in $ui['files']) {
        $relative = [string]$record['path']
        if ($relative.Length -notin 1..512 -or $relative -match '[\\:\x00-\x1f\x7f]' -or
            @($relative.Split('/') | Where-Object { $_ -in @('', '.', '..') }).Count -ne 0 -or
            $record['sha256'] -notmatch '^[a-f0-9]{64}$' -or [string]$record['bytes'] -notmatch '^[0-9]+$') { throw 'Invalid UI asset record' }
        $name = "assets/ui/$relative"
        if (-not $uiNames.Add($name)) { throw 'Duplicate UI asset record' }
        $entry = $archive.GetEntry($name)
        $length = [long]$record['bytes']
        $total += $length
        if ($length -gt 32 * 1024 * 1024 -or $total -gt 32 * 1024 * 1024 -or $null -eq $entry -or $entry.Length -ne $length) { throw "UI asset length mismatch: $relative" }
        $payloadStream = $entry.Open()
        $hash = [Security.Cryptography.SHA256]::Create()
        try {
            $actual = [BitConverter]::ToString($hash.ComputeHash($payloadStream)).Replace('-', '').ToLowerInvariant()
            if ($actual -ne $record['sha256']) { throw "UI checksum mismatch: $relative" }
        } finally { $payloadStream.Dispose(); $hash.Dispose() }
    }
    if (-not $uiNames.Contains('assets/ui/index.html')) { throw 'Missing shared index.html' }
    foreach ($entry in $archive.Entries) {
        if ($entry.FullName -like 'assets/ui/*' -and -not $uiNames.Contains($entry.FullName)) { throw 'Unlisted UI asset' }
    }
    $expectedManifest = [IO.File]::ReadAllText("$PSScriptRoot/app/build/generated/webAssets/ui-manifest.json")
    if ($text -cne $expectedManifest) { throw 'Packaged UI manifest differs from generated build input' }
    Write-Output "Shared UI $($ui['id']): $($uiNames.Count) assets / $total bytes; all SHA-256 verified"
} finally {
    $archive.Dispose()
    foreach ($name in @('libtonalli_core.so', 'libtonalli_maps.so', 'libjnidispatch.so')) {
        $extracted = Join-Path $inspection $name
        if (Test-Path -LiteralPath $extracted) { Remove-Item -LiteralPath $extracted }
    }
    foreach ($dexFile in @(Get-ChildItem -LiteralPath $inspection -File -Filter 'classes*.dex')) {
        Remove-Item -LiteralPath $dexFile.FullName
    }
    Remove-Item -LiteralPath $inspection
}
& "$sdk/build-tools/35.0.0/zipalign.exe" -c -P 16 4 $apkPath
if ($LASTEXITCODE -ne 0) { throw 'APK ZIP alignment failed' }
$previousJava = $env:JAVA_HOME
try {
    if ($env:TONALLI_CI_TOOLS -ne '1') { $env:JAVA_HOME = Join-Path $env:ProgramFiles 'Microsoft/jdk-17.0.20.101-hotspot' }
    & "$sdk/build-tools/35.0.0/apksigner.bat" -JXmx256M verify --verbose --print-certs $apkPath
    if ($LASTEXITCODE -ne 0) { throw 'APK signature verification failed' }
} finally { $env:JAVA_HOME = $previousJava }
$badging | Select-String 'package:|sdkVersion|targetSdkVersion|application-label:|native-code:'
$stream = [IO.File]::OpenRead($apkPath)
$sha = [Security.Cryptography.SHA256]::Create()
try {
    $digest = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
    Write-Output "SHA256 $digest  $apkPath"
} finally { $stream.Dispose(); $sha.Dispose() }
