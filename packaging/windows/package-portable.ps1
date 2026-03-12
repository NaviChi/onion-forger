param(
  [Parameter(Mandatory = $true)]
  [string]$Tag
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$portableRoot = "portable-assets/crawli-windows-x64-portable"
$portableBin = Join-Path $portableRoot "bin"

if (Test-Path $portableRoot) {
  Remove-Item -Path $portableRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $portableBin | Out-Null

$exe = Get-ChildItem -Path "src-tauri/target" -Recurse -Filter "crawli.exe" |
  Where-Object { $_.FullName -match "\\release\\crawli\.exe$" -and $_.FullName -notmatch "\\deps\\" } |
  Select-Object -First 1
if (-not $exe) {
  throw "Portable build failed: crawli.exe was not found in src-tauri/target/**/release/"
}

$cliExe = Get-ChildItem -Path "src-tauri/target" -Recurse -Filter "crawli-cli.exe" |
  Where-Object { $_.FullName -match "\\release\\crawli-cli\.exe$" -and $_.FullName -notmatch "\\deps\\" } |
  Select-Object -First 1
if (-not $cliExe) {
  throw "Portable build failed: crawli-cli.exe was not found in src-tauri/target/**/release/"
}

Copy-Item -Path $exe.FullName -Destination (Join-Path $portableRoot "crawli.exe") -Force
Copy-Item -Path $cliExe.FullName -Destination (Join-Path $portableRoot "crawli-cli.exe") -Force

$cmdShim = "@echo off`r`n`"%~dp0crawli-cli.exe`" %*"
Set-Content -Path (Join-Path $portableRoot "crawli-cli.cmd") -Value $cmdShim -Encoding ascii

if (Test-Path "packaging/windows/README_PORTABLE.txt") {
  Copy-Item -Path "packaging/windows/README_PORTABLE.txt" -Destination (Join-Path $portableRoot "README.txt") -Force
}

if (Test-Path "src-tauri/bin/win_x64") {
  Copy-Item -Path "src-tauri/bin/win_x64" -Destination $portableBin -Recurse -Force
}

$zipPath = "portable-assets/crawli_${Tag}_windows_x64_portable.zip"
if (Test-Path $zipPath) {
  Remove-Item -Path $zipPath -Force
}
Compress-Archive -Path "$portableRoot/*" -DestinationPath $zipPath

$zipPath
