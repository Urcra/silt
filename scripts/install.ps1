# Installs `silt.exe` from a GitHub release on Windows (ADR 0045):
#   irm https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.ps1 | iex
#
#   SILT_VERSION       a release tag such as v0.1.0; the latest by default
#   SILT_INSTALL_DIR   where `silt.exe` goes; %LOCALAPPDATA%\Programs\silt by default
#   SILT_DOWNLOAD_URL  where the archives and SHA256SUMS are, in place of
#                      the release's own URL
$ErrorActionPreference = 'Stop'

$repo = 'https://github.com/Urcra/silt'
$dir = if ($env:SILT_INSTALL_DIR) { $env:SILT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\silt' }

# A throw rather than `exit`, which would close the window `iex` runs in.
function Fail([string] $message) {
    throw "silt install: $message"
}

$target = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64-pc-windows-msvc' }
    'ARM64' { 'aarch64-pc-windows-msvc' }
    default { Fail "no release binary for $($env:PROCESSOR_ARCHITECTURE)" }
}

$url = if ($env:SILT_DOWNLOAD_URL) {
    $env:SILT_DOWNLOAD_URL
} elseif ($env:SILT_VERSION) {
    "$repo/releases/download/$($env:SILT_VERSION)"
} else {
    "$repo/releases/latest/download"
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("silt-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
try {
    $archive = "silt-$target.zip"
    Write-Host "silt install: downloading $url/$archive"
    try {
        Invoke-WebRequest -UseBasicParsing "$url/$archive" -OutFile (Join-Path $work $archive)
        Invoke-WebRequest -UseBasicParsing "$url/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS')
    } catch {
        Fail "could not download $url/$archive"
    }
    $expected = Get-Content (Join-Path $work 'SHA256SUMS') |
        ForEach-Object { $fields = -split $_; if ($fields[1] -eq $archive -or $fields[1] -eq "*$archive") { $fields[0] } } |
        Select-Object -First 1
    if (-not $expected) { Fail "SHA256SUMS lists no $archive" }
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $work $archive)).Hash
    if ($actual -ne $expected.ToUpperInvariant()) { Fail "$archive doesn't match its checksum; nothing installed" }

    Expand-Archive (Join-Path $work $archive) -DestinationPath $work
    $exe = Join-Path $work 'silt.exe'
    if (-not (Test-Path $exe)) { Fail "$archive holds no silt.exe" }
    New-Item -ItemType Directory -Force $dir | Out-Null
    # A running silt.exe can't be overwritten, but it can be renamed away.
    $installed = Join-Path $dir 'silt.exe'
    $old = Join-Path $dir 'silt.old.exe'
    Remove-Item -Force -ErrorAction SilentlyContinue $old
    if (Test-Path $installed) { Move-Item -Force $installed $old }
    Copy-Item $exe $installed
    Write-Host "silt install: installed $(& $installed --version) to $installed"
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $work
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $dir)) {
    [Environment]::SetEnvironmentVariable('Path', ($(if ($userPath) { "$userPath;" } else { '' }) + $dir), 'User')
    Write-Host "silt install: added $dir to your PATH; open a new terminal to use it"
}
Write-Host 'silt install: next, import the sessions you already have: silt import'
