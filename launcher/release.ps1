<#
.SYNOPSIS
  Builds the signed launcher installer and, if asked, uploads it to the server.
.EXAMPLE
  .\release.ps1
.EXAMPLE
  .\release.ps1 -Server http://my-server:7878 -AdminToken xxxxxxxx
#>
param(
  [string]$Server,
  [string]$AdminToken,
  [string]$Key = (Join-Path $PSScriptRoot '..\oedvi-updater.key'),
  # Where a standalone launcher looks for updates (the latest.json of your
  # GitHub releases, for instance). Without it, only remote-mode launchers
  # update themselves (from their server).
  [string]$UpdateUrl
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# A tool installed a minute ago is not on the PATH of this terminal yet.
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
foreach ($tool in 'node', 'cargo') {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "$tool not found: install Node.js LTS and Rust (rustup), then reopen PowerShell." }
}

if (-not (Test-Path 'node_modules\.bin\tauri.cmd')) {
  npm ci
  if ($LASTEXITCODE) { throw 'npm ci failed' }
}

if (-not (Test-Path $Key)) {
  Write-Host "Creating the signing key $Key (no password). Keep it: without it, no update can be published anymore."
  npx tauri signer generate --ci -w $Key
  if ($LASTEXITCODE) { throw 'creating the key failed' }
}

# The CLI checks the public key in the config while it signs.
node scripts/release-config.mjs "$Key.pub"
if ($LASTEXITCODE) { throw 'writing the release config failed' }

$env:OEDVI_UPDATER_PUBKEY = (Get-Content "$Key.pub" -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $Key -Raw).Trim()
if ($null -eq $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '' }
if ($UpdateUrl) { $env:OEDVI_UPDATE_URL = $UpdateUrl }

npx tauri build --bundles nsis --config src-tauri/tauri.release.local.conf.json
if ($LASTEXITCODE) { throw 'the build failed' }

$dir = Join-Path $PSScriptRoot '..\target\release\bundle\nsis'
$setup = Get-ChildItem $dir -Filter '*_x64-setup.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw "installer not found in $dir" }
$sig = "$($setup.FullName).sig"
if (-not (Test-Path $sig)) { throw "signature missing: $sig" }
Write-Host "Installer : $($setup.FullName)"
Write-Host "Signature : $sig"

if ($Server) {
  if (-not $AdminToken) { throw '-AdminToken is required with -Server' }
  foreach ($f in @($setup.FullName, $sig)) {
    $name = Split-Path $f -Leaf
    curl.exe --fail -sS -X PUT -H "Authorization: Bearer $AdminToken" --data-binary "@$f" "$($Server.TrimEnd('/'))/api/launcher/files/$name"
    if ($LASTEXITCODE) { throw "uploading $name failed" }
    Write-Host ""
  }
  Write-Host 'Uploaded. Launchers will offer the update at their next check.'
}
