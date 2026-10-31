# Tests install-server.ps1 against a fake release, on Windows:
#   powershell -NoProfile -ExecutionPolicy Bypass -File server\test-install.ps1
# Needs rustc for a stub binary. Touches nothing outside a temp folder.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("amber-install-test-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
$failures = 0
function Fail([string]$why) { Write-Host "FAIL: $why"; $script:failures++ }

try {
    # A stub that writes down how it was called.
    $src = Join-Path $tmp 'stub.rs'
    Set-Content -LiteralPath $src -Value @'
use std::io::Write;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") { println!("amber-server 0.0.0"); return; }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(std::env::var("AMBER_STUB_CALLS").unwrap()).unwrap();
    writeln!(f, "{}", args.join(" ")).unwrap();
    if args.join(" ") == "service install" && std::env::var("STUB_NO_TASK").is_ok() { std::process::exit(1); }
}
'@
    & rustc -O -o (Join-Path $tmp 'stub.exe') $src
    if ($LASTEXITCODE -ne 0) { throw 'rustc failed' }

    function New-Release([bool]$goodSum) {
        $rel = Join-Path $tmp 'release'
        Remove-Item -LiteralPath $rel, (Join-Path $tmp 'local'), (Join-Path $tmp 'calls') -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Path $rel, (Join-Path $tmp 'local') | Out-Null
        $asset = 'amber-server-windows-x64-cpu-compat.exe'
        Copy-Item -LiteralPath (Join-Path $tmp 'stub.exe') -Destination (Join-Path $rel $asset)
        $hash = if ($goodSum) { (Get-FileHash -Algorithm SHA256 (Join-Path $rel $asset)).Hash.ToLower() } else { '0' * 64 }
        Set-Content -LiteralPath (Join-Path $rel 'SHA256SUMS') -Value "$hash  $asset"
        $rel
    }

    function Install([string]$rel) {
        $env:AMBER_SERVER_BASE = $rel
        $env:AMBER_SERVER_FORCE_BUILD = 'cpu-compat'
        $env:AMBER_SERVER_KEEP_PATH = '1'
        $env:AMBER_STUB_CALLS = Join-Path $tmp 'calls'
        $saved = $env:LOCALAPPDATA
        $env:LOCALAPPDATA = Join-Path $tmp 'local'
        try { (& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $here 'install-server.ps1') *>&1 | Out-String) }
        finally { $env:LOCALAPPDATA = $saved }
    }

    $bin = Join-Path $tmp 'local\Programs\amber-server\amber-server.exe'

    # 1. Installs, sets up, starts the task, shows the address.
    $out = Install (New-Release $true)
    if (-not (Test-Path $bin)) { Fail "not installed: $out" }
    $calls = (Get-Content (Join-Path $tmp 'calls') -ErrorAction SilentlyContinue) -join ','
    if ($calls -ne 'setup --quick,service install,url') { Fail "the commands were: $calls" }
    if ($out -notmatch 'amber-server cleanup add') { Fail 'no hint about the cleanup' }

    # 2. Again, as an update: replaces the binary.
    $out = Install (New-Release $true)
    if ($out -match 'install:') { Fail "the update failed: $out" }

    # 3. A wrong checksum installs nothing.
    $out = Install (New-Release $false)
    if ($out -notmatch 'does not match its checksum') { Fail "no checksum error: $out" }
    if (Test-Path $bin) { Fail 'installed despite a wrong checksum' }
    if (Test-Path (Join-Path $tmp 'calls')) { Fail 'ran despite a wrong checksum' }

    # 4. Without the task the install still ends with the address and how to start by hand.
    $env:STUB_NO_TASK = '1'
    $out = Install (New-Release $true)
    Remove-Item Env:STUB_NO_TASK
    if ($out -notmatch 'amber-server.exe run') { Fail "no hint to start it by hand: $out" }
    if ((Get-Content (Join-Path $tmp 'calls'))[-1] -ne 'url') { Fail 'the address was not shown' }
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failures -eq 0) { Write-Host 'install tests: ok' } else { exit 1 }
