# Real PowerShell contract test for the hosted Windows cache normalizer.
# Shadows rustup/rustc with functions: never touches the machine's toolchains.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script:normalizer = Join-Path $PSScriptRoot 'ci_windows_rust_cache.ps1'
$script:pinned = '1.99.1-x86_64-pc-windows-msvc'
$script:unrelated = '1.99.0-x86_64-pc-windows-msvc'
$script:selected = $script:pinned
$script:release = '1.99.1'
$script:inventory = @($script:pinned, $script:unrelated)
$script:removed = [System.Collections.Generic.List[string]]::new()

function rustc {
    if (($args -join ' ') -ne '-vV') { throw "Unexpected fake rustc call: $args" }
    $global:LASTEXITCODE = 0
    "rustc $script:release (fixture)", "release: $script:release", 'host: x86_64-pc-windows-msvc'
}

function rustup {
    $command = $args -join ' '
    $global:LASTEXITCODE = 0
    if ($command -eq 'show active-toolchain') { return "$script:selected (override fixture)" }
    if ($command -eq 'toolchain list --quiet') { return $script:inventory }
    if ($command -eq "run $script:selected rustc -vV") {
        return "rustc $script:release (fixture)", "release: $script:release", 'host: x86_64-pc-windows-msvc'
    }
    if ($command -match '^toolchain uninstall (.+)$') {
        $candidate = $Matches[1]
        if ($candidate -eq $script:pinned) { throw 'Test caught an attempt to remove pinned Rust' }
        $script:removed.Add($candidate)
        $script:inventory = @($script:inventory | Where-Object { $_ -ne $candidate })
        return
    }
    throw "Unexpected fake rustup call: $command"
}

if ((Get-Command rustup).CommandType -ne 'Function' -or
    (Get-Command rustc).CommandType -ne 'Function') {
    throw 'Mock functions must shadow real rustup/rustc before running the script'
}

$env:GITHUB_ACTIONS = 'true'
$env:RUNNER_ENVIRONMENT = 'github-hosted'
$env:RUNNER_OS = 'Windows'
$env:ImageOS = 'win22'
$env:ImageVersion = 'test-image-1'

function Assert-RefusedBeforeRemoval([string]$reason, [string]$pattern) {
    $refused = $false
    try { . $script:normalizer }
    catch {
        if ($_.Exception.Message -notmatch $pattern) {
            throw "Unexpected refusal for $reason`: $($_.Exception.Message)"
        }
        $refused = $true
    }
    if (-not $refused) { throw "Wrongly accepted $reason" }
    if ($script:removed.Count -ne 0) { throw "Removed Rust toolchains despite $reason" }
    Write-Host "PASS (refusal): $reason"
}

# Failure-first negatives: every failure must precede ANY uninstall operation.
$script:selected = $script:unrelated
Assert-RefusedBeforeRemoval 'unselected active toolchain' 'Wrong active Rust compiler'
$script:selected = $script:pinned
$script:release = '1.99.0'
Assert-RefusedBeforeRemoval 'wrong rustc release' 'does not match pinned version'
$script:release = '1.99.1'
$env:RUNNER_ENVIRONMENT = 'self-hosted'
Assert-RefusedBeforeRemoval 'developer or self-hosted machine' 'disposable GitHub-hosted Windows runner'
$env:RUNNER_ENVIRONMENT = 'github-hosted'
$env:ImageVersion = ''
Assert-RefusedBeforeRemoval 'unknown runner image' 'Missing ImageOS/ImageVersion'
$env:ImageVersion = 'test-image-1'

# Positive: remove ONLY the irrelevant preinstalled toolchain.
. $script:normalizer
if ($script:removed.Count -ne 1 -or $script:removed[0] -ne $script:unrelated -or
    $script:inventory.Count -ne 1 -or $script:inventory[0] -ne $script:pinned) {
    throw 'Normalization did not retain exactly the pinned compiler'
}
Write-Host 'PASS: selected compiler preserved; irrelevant toolchain removed'
