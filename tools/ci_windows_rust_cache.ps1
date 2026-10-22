# Runs only on a disposable GitHub-hosted Windows runner, after the pinned
# dtolnay/rust-toolchain action and before Swatinem/rust-cache. Never run this
# against a developer's Windows installation or a self-hosted runner: it removes
# runner-preinstalled Rust toolchains that are not selected by the project.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -ne 'true' -or
    $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:RUNNER_OS -ne 'Windows') {
    throw 'Rust cache toolchain normalization requires a disposable GitHub-hosted Windows runner'
}
if ([string]::IsNullOrWhiteSpace($env:ImageOS) -or
    [string]::IsNullOrWhiteSpace($env:ImageVersion)) {
    throw 'Missing ImageOS/ImageVersion: cache would not invalidate on a Windows runner image change'
}

$manifestPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'rust-toolchain.toml'
$toolchainManifest = [System.IO.File]::ReadAllText($manifestPath)
if ($toolchainManifest -notmatch '(?m)^\[toolchain\][ \t]*$') {
    throw 'rust-toolchain.toml is missing the selected [toolchain] table'
}
$pinMatches = [regex]::Matches($toolchainManifest, '(?m)^\s*channel\s*=\s*"(?<version>\d+\.\d+\.\d+)"\s*(?:#.*)?$')
if ($pinMatches.Count -ne 1) { throw 'Expected one exact Rust compiler channel in rust-toolchain.toml' }
$pin = $pinMatches[0].Groups['version'].Value

$actualCompiler = @(& rustc -vV 2>&1)
if ($LASTEXITCODE -ne 0) { throw 'Active rustc -vV failed' }
$releaseLines = @($actualCompiler | Where-Object { $_.ToString().StartsWith('release: ') })
if ($releaseLines.Count -ne 1 -or $releaseLines[0] -ne "release: $pin") {
    throw "Active Rust compiler does not match pinned version $pin"
}
$hostLines = @($actualCompiler | Where-Object { $_.ToString().StartsWith('host: ') })
if ($hostLines.Count -ne 1) { throw 'Cannot identify active Rust target triple' }
$compilerHost = $hostLines[0].ToString().Substring('host: '.Length).Trim()
if ($compilerHost -notmatch '^[A-Za-z0-9_-]+$') { throw 'Invalid Rust target triple' }
$expectedToolchain = "$pin-$compilerHost"

$activeOutput = @(& rustup show active-toolchain 2>&1)
if ($LASTEXITCODE -ne 0 -or $activeOutput.Count -ne 1) {
    throw 'Cannot identify the active compiler selected by rust-toolchain.toml'
}
$active = ($activeOutput[0].ToString().Trim() -split '\s+')[0]
if ([string]::IsNullOrWhiteSpace($active)) { throw 'Active Rust toolchain name is empty' }
if ($active -ne $expectedToolchain) { throw "Wrong active Rust compiler: expected $expectedToolchain, got $active" }
$explicitCompiler = @(& rustup run $active rustc -vV 2>&1)
if ($LASTEXITCODE -ne 0 -or ($actualCompiler -join "`n") -ne ($explicitCompiler -join "`n")) {
    throw "Selected rustc differs from the pinned compiler $active"
}

$beforeOutput = @(& rustup toolchain list --quiet 2>&1)
if ($LASTEXITCODE -ne 0) { throw 'Failed to list installed Rust toolchains' }
$before = @($beforeOutput | ForEach-Object { ($_.ToString().Trim() -split '\s+')[0] } | Where-Object { $_ -ne '' })
if ($before -notcontains $active) { throw "Active Rust toolchain $active is missing from inventory" }

foreach ($toolchain in $before) {
    if ($toolchain -eq $active) { continue }
    & rustup toolchain uninstall $toolchain
    if ($LASTEXITCODE -ne 0) { throw "Could not uninstall unrelated hosted toolchain $toolchain" }
}

$afterOutput = @(& rustup toolchain list --quiet 2>&1)
if ($LASTEXITCODE -ne 0) { throw 'Failed to check normalized Rust toolchain inventory' }
$after = @($afterOutput | ForEach-Object { ($_.ToString().Trim() -split '\s+')[0] } | Where-Object { $_ -ne '' })
if ($after.Count -ne 1 -or $after[0] -ne $active) {
    throw "Cache Rust toolchain inventory must contain only $active; found: $($after -join ', ')"
}

Write-Host "Rust cache compiler: $active; hosted image: $env:ImageOS/$env:ImageVersion"
Write-Host ($actualCompiler -join "`n")
