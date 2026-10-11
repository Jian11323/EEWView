# 转调 Python 便携打包（避免 Copy-Item / Compress-Archive 卡死）
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root
python (Join-Path $Root "tools\pack_windows.py") @args
exit $LASTEXITCODE
