$ErrorActionPreference = 'Stop'
$GradleArguments = @($args)
$previousJava = $env:JAVA_HOME
$previousSdk = $env:ANDROID_HOME
try {
    if ($env:TONALLI_CI_TOOLS -ne '1') {
        $env:JAVA_HOME = Join-Path $env:ProgramFiles 'Microsoft/jdk-17.0.20.101-hotspot'
        $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA 'Android/Sdk'
    }
    if (-not (Test-Path "$env:JAVA_HOME/bin/java.exe")) { throw 'Install JDK 17 or update this local driver to its exact verified path.' }
    & "$PSScriptRoot/gradlew.bat" --project-dir "$PSScriptRoot" @GradleArguments
    exit $LASTEXITCODE
} finally {
    $env:JAVA_HOME = $previousJava
    $env:ANDROID_HOME = $previousSdk
}
