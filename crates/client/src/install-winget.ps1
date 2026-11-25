# Installs winget (“App Installer”)
# for the current user. Run by the launcher when winget is missing; needs no
# administrator rights. Exit code 0 only when winget.exe is there afterwards.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\winget.exe'
# Reparse points are invisible to Test-Path -PathType Leaf on some builds.
function Has-Winget { [bool](Get-Item -LiteralPath $alias -Force -ErrorAction SilentlyContinue) }

if (Has-Winget) { Write-Output 'winget is already there.'; exit 0 }

# 1. App Installer ships with Windows 10 1809+ and 11 but is not registered
#    for a user who has never signed in to the Store (fresh profile, sandbox).
try {
    Write-Output 'Registering App Installer...'
    Add-AppxPackage -RegisterByFamilyName -MainPackage 'Microsoft.DesktopAppInstaller_8wekyb3d8bbwe'
    Start-Sleep -Seconds 3
    if (Has-Winget) { Write-Output 'winget is ready.'; exit 0 }
} catch {
    Write-Output ('  not possible: ' + $_.Exception.Message)
}

# 2. Download the package and its dependencies from Microsoft.
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
$tmp = Join-Path $env:TEMP 'oedvi-winget'
try {
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $bundle = Join-Path $tmp 'AppInstaller.msixbundle'
    $zip = Join-Path $tmp 'dependencies.zip'
    Write-Output 'Downloading App Installer (Microsoft)...'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://aka.ms/getwinget' -OutFile $bundle
    Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/microsoft/winget-cli/releases/latest/download/DesktopAppInstallerDependencies.zip' -OutFile $zip
    Expand-Archive -Force -Path $zip -DestinationPath (Join-Path $tmp 'dependencies')
    $deps = @(Get-ChildItem -Path (Join-Path $tmp 'dependencies') -Recurse -Filter '*.appx' |
        Where-Object { $_.FullName -match "[\\/]$arch[\\/]" } | ForEach-Object { $_.FullName })
    Write-Output ('Installing (' + $deps.Count + ' dependencies)...')
    if ($deps.Count -gt 0) {
        Add-AppxPackage -Path $bundle -DependencyPath $deps
    } else {
        Add-AppxPackage -Path $bundle
    }
} finally {
    Remove-Item -Recurse -Force -Path $tmp -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 3
if (Has-Winget) { Write-Output 'winget is ready.'; exit 0 }
Write-Output 'winget is still not found after the install.'
exit 1
