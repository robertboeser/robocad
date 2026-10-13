# Builds a zip with robocad.exe and robocad-cli.exe in dist\.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches.Groups[1].Value
cargo build --release -p rc-app -p rc-cli
$name = "robocad-$version-windows-x86_64"
$stage = "dist\$name"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\robocad.exe, target\release\robocad-cli.exe, README.md, LICENSE-MIT, LICENSE-APACHE $stage
Compress-Archive -Path $stage -DestinationPath "dist\$name.zip" -Force
Write-Output "built dist\$name.zip"
