# Install the FrisyDisk terminal app (frisy) for this user on Windows:
#   irm https://raw.githubusercontent.com/agentic-work/frisydisk/main/scripts/install.ps1 | iex
# Installs into %LOCALAPPDATA%\FrisyDisk\cli and adds it to your PATH. No administrator rights needed.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = 'agentic-work/frisydisk'

$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'frisy-install' }
$asset = $release.assets | Where-Object { $_.name -like 'frisy-cli-*-windows-x64.zip' } | Select-Object -First 1
if (-not $asset) { throw "The latest release has no Windows build of frisy." }

$dest = Join-Path $env:LOCALAPPDATA 'FrisyDisk\cli'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("frisy-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading $($asset.name)"
    $zip = Join-Path $tmp 'frisy.zip'
    Invoke-WebRequest $asset.browser_download_url -OutFile $zip -Headers @{ 'User-Agent' = 'frisy-install' }
    Expand-Archive $zip -DestinationPath $tmp -Force
    if (Test-Path $dest) { Remove-Item $dest -Recurse -Force }
    New-Item -ItemType Directory -Path $dest -Force | Out-Null
    Copy-Item (Join-Path $tmp 'frisy\*') $dest -Recurse -Force
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dest) {
    [Environment]::SetEnvironmentVariable('Path', ($(if ($userPath) { "$userPath;" } else { '' }) + $dest), 'User')
    Write-Host "Added $dest to your PATH. Open a new terminal to use it."
}
$env:Path = "$env:Path;$dest"
Write-Host "Installed frisy to $dest"
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host "Install Node.js 18 or newer for the interactive view:  winget install OpenJS.NodeJS.LTS"
}
Write-Host "Try:  frisy C:\      frisy clean      frisy du -c $env:USERPROFILE"
