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
    match args.first().map(String::as_str) {
        Some("--version") => { println!("amber-server 0.0.0"); return; }
        Some("gpu-check") => std::process::exit(if std::env::var("STUB_NO_GPU").is_ok() { 1 } else { 0 }),
        Some("status") => {
            println!("server   {}", if std::env::var("STUB_DOWN").is_ok() { "not running" } else { "answers on port 8765" });
            return;
        }
        Some("logs") => { println!("amber-server: port 8765 is in use"); return; }
        _ => {}
    }
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
        $lines = @()
        $marker = 0
        foreach ($build in 'cpu', 'cpu-compat', 'vulkan') {
            $asset = "amber-server-windows-x64-$build.exe"
            # One byte more at the end tells the builds apart.
            $bytes = [IO.File]::ReadAllBytes((Join-Path $tmp 'stub.exe')) + [byte]$marker
            [IO.File]::WriteAllBytes((Join-Path $rel $asset), $bytes)
            $marker++
            $hash = if ($goodSum) { (Get-FileHash -Algorithm SHA256 (Join-Path $rel $asset)).Hash.ToLower() } else { '0' * 64 }
            $lines += "$hash  $asset"
        }
        Set-Content -LiteralPath (Join-Path $rel 'SHA256SUMS') -Value $lines
        $rel
    }

    function Install([string]$rel, [string]$force = 'cpu-compat', [string]$flags = 'avx avx2') {
        $env:AMBER_SERVER_BASE = $rel
        $env:AMBER_SERVER_FORCE_BUILD = $force
        $env:AMBER_SERVER_CPU_FLAGS = $flags
        $env:AMBER_SERVER_WAIT = '1'
        $env:AMBER_SERVER_KEEP_PATH = '1'
        $env:AMBER_STUB_CALLS = Join-Path $tmp 'calls'
        $saved = $env:LOCALAPPDATA
        $env:LOCALAPPDATA = Join-Path $tmp 'local'
        try { (& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $here 'install-server.ps1') *>&1 | Out-String) }
        finally { $env:LOCALAPPDATA = $saved }
    }

    $bin = Join-Path $tmp 'local\Programs\amber-server\amber-server.exe'
    function Installed {
        $h = (Get-FileHash -Algorithm SHA256 $bin).Hash
        foreach ($build in 'cpu', 'cpu-compat', 'vulkan') {
            if ((Get-FileHash -Algorithm SHA256 (Join-Path $tmp "release\amber-server-windows-x64-$build.exe")).Hash -eq $h) { return $build }
        }
        'unknown'
    }

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

    # 5. The GPU build seeing no GPU gives way to the CPU build.
    $env:STUB_NO_GPU = '1'
    $out = Install (New-Release $true) 'vulkan' 'avx avx2'
    Remove-Item Env:STUB_NO_GPU
    if ((Installed) -ne 'cpu') { Fail "no GPU should give the cpu build, got $(Installed): $out" }
    $out = Install (New-Release $true) 'vulkan' 'avx avx2'
    if ((Installed) -ne 'vulkan') { Fail "a GPU should keep the vulkan build, got $(Installed): $out" }

    # 6. A CPU without AVX is told, and nothing is installed.
    $out = Install (New-Release $true) '' 'sse4_2'
    if ($out -notmatch 'no AVX') { Fail "no word about AVX: $out" }
    if (Test-Path $bin) { Fail 'installed on a CPU without AVX' }

    # 7. A server that does not answer after the install is reported, with its log.
    $env:STUB_DOWN = '1'
    $out = Install (New-Release $true)
    Remove-Item Env:STUB_DOWN
    if ($out -notmatch 'port 8765 is in use') { Fail "the log was not shown: $out" }
    if ($out -notmatch 'does not answer') { Fail "not reported: $out" }
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failures -eq 0) { Write-Host 'install tests: ok' } else { exit 1 }
