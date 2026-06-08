# borehole CLI installer for Windows (PowerShell).
#
# Downloads the prebuilt borehole.exe from the GitHub Releases of this
# repository, installs it under %LOCALAPPDATA%\borehole and adds that
# directory to the user PATH so it can be run as `borehole`.
#
# Usage:
#   irm https://raw.githubusercontent.com/llinguini/borehole/main/install.ps1 | iex
#
# Environment overrides:
#   BOREHOLE_REPO     "owner/repo" to install from (default: llinguini/borehole)
#   BOREHOLE_VERSION  tag to install, e.g. v0.1.1 (default: latest)

$ErrorActionPreference = 'Stop'

$repo = if ($env:BOREHOLE_REPO) { $env:BOREHOLE_REPO } else { 'llinguini/borehole' }
$version = if ($env:BOREHOLE_VERSION) { $env:BOREHOLE_VERSION } else { 'latest' }

# Only x86_64 Windows is published today.
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne 'AMD64') {
    throw "arquitectura no soportada: $arch (solo se publica x86_64 Windows)"
}

$asset = 'borehole-x86_64-pc-windows-msvc.exe'
$url = if ($version -eq 'latest') {
    "https://github.com/$repo/releases/latest/download/$asset"
} else {
    "https://github.com/$repo/releases/download/$version/$asset"
}

$installDir = Join-Path $env:LOCALAPPDATA 'borehole'
$dest = Join-Path $installDir 'borehole.exe'

Write-Host "Descargando $asset ($version)..."
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Invoke-WebRequest -Uri $url -OutFile $dest

Write-Host "✓ borehole instalado en $dest"

# Add the install directory to the user PATH if it is not already present.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not ($userPath -split ';' | Where-Object { $_ -eq $installDir })) {
    $newPath = if ([string]::IsNullOrEmpty($userPath)) {
        $installDir
    } else {
        "$userPath;$installDir"
    }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host ""
    Write-Host "Añadido $installDir a tu PATH de usuario."
    Write-Host "Reinicia la terminal para aplicar el cambio."
}

Write-Host ""
Write-Host "Ejecuta: borehole config"
