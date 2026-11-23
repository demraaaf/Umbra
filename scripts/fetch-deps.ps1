# Downloads the build dependencies into third_party:
#   - FFmpeg 8.1 LGPL shared build (headers, import libraries and DLLs)
#   - libclang, used only at build time to generate the FFmpeg bindings
#
# FFmpeg 8.1 is used instead of 9.0 on purpose: the 9.0 builds need NVIDIA driver 610 or
# newer, while 8.1 works from driver 570.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$thirdParty = Join-Path $root 'third_party'
$downloads = Join-Path $thirdParty 'dl'
New-Item -ItemType Directory -Force $downloads | Out-Null

$ffmpegName = 'ffmpeg-n8.1-latest-win64-lgpl-shared-8.1'
$ffmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/$ffmpegName.zip"
$libclangUrl = 'https://files.pythonhosted.org/packages/0b/2d/3f480b1e1d31eb3d6de5e3ef641954e5c67430d5ac93b7fa7e07589576c7/libclang-18.1.1-py2.py3-none-win_amd64.whl'

$ffmpegDir = Join-Path $thirdParty 'ffmpeg'
if (-not (Test-Path (Join-Path $ffmpegDir 'lib\avcodec.lib'))) {
    $zip = Join-Path $downloads 'ffmpeg-8.1.zip'
    Write-Host "Downloading FFmpeg 8.1 (about 80 MB)"
    Invoke-WebRequest $ffmpegUrl -OutFile $zip
    Expand-Archive $zip -DestinationPath $thirdParty -Force
    if (Test-Path $ffmpegDir) { Remove-Item -Recurse -Force $ffmpegDir }
    Rename-Item (Join-Path $thirdParty $ffmpegName) 'ffmpeg'
}

$libclangDir = Join-Path $thirdParty 'libclang'
if (-not (Test-Path (Join-Path $libclangDir 'libclang.dll'))) {
    # A wheel is a zip file; only libclang.dll is needed from it.
    $zip = Join-Path $downloads 'libclang.zip'
    $unpacked = Join-Path $downloads 'libclang'
    Write-Host "Downloading libclang 18.1.1 (about 26 MB)"
    Invoke-WebRequest $libclangUrl -OutFile $zip
    Expand-Archive $zip -DestinationPath $unpacked -Force
    New-Item -ItemType Directory -Force $libclangDir | Out-Null
    Get-ChildItem $unpacked -Recurse -Filter 'libclang.dll' | Copy-Item -Destination $libclangDir
    Remove-Item -Recurse -Force $unpacked
}

Write-Host "Dependencies are in $thirdParty"
