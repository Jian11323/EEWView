# 组装 Windows 预览目录：exe + assets + geodata + config（不含走时 raw、密钥）。
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

Write-Host "cargo build -p jian-app --release"
cargo build -p jian-app --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Out = Join-Path $Root "dist\EEWView"
if (Test-Path $Out) {
    Remove-Item $Out -Recurse -Force
}
New-Item -ItemType Directory -Path $Out | Out-Null

Copy-Item (Join-Path $Root "target\release\jian.exe") (Join-Path $Out "jian.exe")
Copy-Item (Join-Path $Root "README.md") (Join-Path $Out "README.md")
Copy-Item (Join-Path $Root "LICENSE") (Join-Path $Out "LICENSE") -ErrorAction SilentlyContinue

Copy-Item (Join-Path $Root "config") (Join-Path $Out "config") -Recurse
Get-ChildItem (Join-Path $Out "config") -Filter "local.toml" -ErrorAction SilentlyContinue | Remove-Item -Force

Copy-Item (Join-Path $Root "assets") (Join-Path $Out "assets") -Recurse
$raw = Join-Path $Out "assets\travel\raw"
if (Test-Path $raw) { Remove-Item $raw -Recurse -Force }

Write-Host "copying geodata (may take a while)..."
Copy-Item (Join-Path $Root "geodata") (Join-Path $Out "geodata") -Recurse

Write-Host "packed: $Out"
Write-Host "run: $Out\jian.exe"
