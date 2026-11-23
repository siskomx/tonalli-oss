$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Hosted CI only' }
& rustup toolchain install 1.96.1 --profile minimal
if ($LASTEXITCODE -ne 0) { throw 'Rust installation failed' }
$nasmArchive = Join-Path $env:RUNNER_TEMP 'nasm.zip'
Invoke-WebRequest 'https://www.nasm.us/pub/nasm/releasebuilds/3.01/win64/nasm-3.01-win64.zip' -OutFile $nasmArchive
if ((Get-FileHash $nasmArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne 'e0ba5157007abc7b1a65118a96657a961ddf55f7e3f632ee035366dfce039ca4') { throw 'NASM digest mismatch' }
$nasmTools = Join-Path $env:RUNNER_TEMP 'nasm-tools'
Expand-Archive $nasmArchive $nasmTools
(Get-ChildItem $nasmTools -Filter nasm.exe -Recurse | Select-Object -First 1).DirectoryName | Out-File $env:GITHUB_PATH -Append -Encoding utf8
if (-not (Test-Path 'C:/msys64/usr/bin/make.exe') -or -not (Test-Path 'C:/msys64/usr/bin/perl.exe')) {
    & C:/msys64/usr/bin/pacman.exe --noconfirm -S --needed make perl
    if ($LASTEXITCODE -ne 0) { throw 'MSYS build tools failed' }
}
$sdkManager = Join-Path $env:ANDROID_HOME 'cmdline-tools/latest/bin/sdkmanager.bat'
& $sdkManager 'platforms;android-34' 'build-tools;35.0.0' 'ndk;29.0.14206865'
if ($LASTEXITCODE -ne 0) { throw 'Android SDK installation failed' }
