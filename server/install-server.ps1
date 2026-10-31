# Amber Server - speech to text for Amber on the phone, installed in one command:
#
#   irm https://github.com/EinFabo/amberpc/releases/download/server-latest/install-server.ps1 | iex
#
# Picks the build for this PC (Vulkan GPU, CPU with AVX2, or older CPU), checks it against the
# release's SHA256SUMS, installs it to %LOCALAPPDATA%\Programs\amber-server, puts it on the
# user's PATH, sets it up with the recommended model, starts it at log-on and shows the
# phone's address. No admin rights needed. Run it again to update; settings and models stay.
#
# $env:AMBER_SERVER_FORCE_BUILD = 'cpu' | 'cpu-compat' | 'vulkan' picks the build yourself;
# $env:AMBER_SERVER_KEEP_PATH = '1' leaves the user's PATH alone.

function Install-AmberServer {
    # Inside a function: with `irm | iex`, `exit` would close the user's window.
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue' # Invoke-WebRequest is many times slower with it
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $base = if ($env:AMBER_SERVER_BASE) { $env:AMBER_SERVER_BASE } else { 'https://github.com/EinFabo/amberpc/releases/download/server-latest' }
    $dest = Join-Path $env:LOCALAPPDATA 'Programs\amber-server'
    $bin = Join-Path $dest 'amber-server.exe'

    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'Amber Server needs 64-bit Windows.'
    }

    if ($null -ne $env:AMBER_SERVER_CPU_FLAGS) {
        # For the installer's own test: the CPU's flags, e.g. 'avx avx2'.
        $flags = $env:AMBER_SERVER_CPU_FLAGS -split '\s+'
        $avx = $flags -contains 'avx'
        $avx2 = $flags -contains 'avx2'
    } else {
        Add-Type -Namespace AmberInstall -Name Cpu -MemberDefinition '[DllImport("kernel32.dll")] public static extern bool IsProcessorFeaturePresent(uint feature);'
        $avx = [AmberInstall.Cpu]::IsProcessorFeaturePresent(39) # PF_AVX_INSTRUCTIONS_AVAILABLE
        $avx2 = [AmberInstall.Cpu]::IsProcessorFeaturePresent(40) # PF_AVX2_INSTRUCTIONS_AVAILABLE
    }
    $build = $env:AMBER_SERVER_FORCE_BUILD
    if (-not $build -and -not $avx) {
        throw 'This CPU has no AVX, which Amber Server needs - it cannot run on this PC.'
    }
    $cpuBuild = if ($avx2) { 'cpu' } else { 'cpu-compat' }

    if (-not $build) {
        # The Vulkan loader and a GPU from NVIDIA or AMD - and AVX2, which the Vulkan build is built for.
        $loader = Test-Path (Join-Path $env:SystemRoot 'System32\vulkan-1.dll')
        $gpu = Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'NVIDIA|AMD|Radeon' }
        $build = if ($avx2 -and $loader -and $gpu) { 'vulkan' } else { $cpuBuild }
    }
    if ($build -notin 'cpu', 'cpu-compat', 'vulkan') {
        throw "Unknown build '$build' - cpu, cpu-compat or vulkan."
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("amber-server-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        function Get-File([string]$name) {
            $out = Join-Path $tmp $name
            if ($base -match '^https?://') {
                Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $out
            } else {
                Copy-Item -LiteralPath (Join-Path $base $name) -Destination $out
            }
            $out
        }

        # Downloads and checks one build; returns its path.
        function Get-Build([string]$which) {
            $asset = "amber-server-windows-x64-$which.exe"
            Write-Host "Downloading $asset ..."
            $sums = Get-Content -LiteralPath (Get-File 'SHA256SUMS')
            $line = $sums | Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($asset))$" } | Select-Object -First 1
            if (-not $line) { throw "SHA256SUMS has no line for $asset." }
            $want = ($line -split '\s+')[0].ToLower()
            $file = Get-File $asset
            $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLower()
            if ($got -ne $want) { throw "$asset does not match its checksum - not installed." }
            $file
        }

        $file = Get-Build $build
        if ($build -eq 'vulkan') {
            # The Vulkan build needs a working driver: it says whether it sees a GPU.
            & $file gpu-check *> $null
            if ($LASTEXITCODE -ne 0) {
                Write-Host 'The GPU build sees no usable GPU here (driver?) - taking the CPU build.'
                $build = $cpuBuild
                $file = Get-Build $build
            }
        }

        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        if (Test-Path -LiteralPath $bin) {
            # A running server keeps its file locked, but Windows lets it be renamed.
            Remove-Item -LiteralPath "$bin.old" -Force -ErrorAction SilentlyContinue
            try { Remove-Item -LiteralPath $bin -Force } catch { Rename-Item -LiteralPath $bin -NewName 'amber-server.exe.old' }
        }
        Copy-Item -LiteralPath $file -Destination $bin
        Write-Host "Installed $bin ($build build)"
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $env:AMBER_SERVER_KEEP_PATH -and -not (($userPath -split ';') -contains $dest)) {
        $joined = if ($userPath) { "$userPath;$dest" } else { $dest }
        [Environment]::SetEnvironmentVariable('Path', $joined, 'User')
        Write-Host 'amber-server is on your PATH from the next terminal on.'
    }
    if (-not (($env:Path -split ';') -contains $dest)) { $env:Path = "$env:Path;$dest" }
    Write-Host ''

    & $bin setup --quick
    if ($LASTEXITCODE -ne 0) { throw 'The setup failed - see above.' }
    Write-Host ''
    Write-Host 'If Windows asks about the firewall: allow private networks, so the phone can reach this PC.'
    & $bin service install
    if ($LASTEXITCODE -eq 0) {
        # Up and answering? Loading the model takes a moment.
        $wait = if ($env:AMBER_SERVER_WAIT) { [int]$env:AMBER_SERVER_WAIT } else { 20 }
        $waited = 0
        while (-not ((& $bin status 2>$null) -match 'answers on port')) {
            if ($waited -ge $wait) {
                Write-Host ''
                Write-Host 'The server was installed but does not answer. Its last lines:'
                & $bin logs 2>&1 | Select-Object -Last 5 | ForEach-Object { Write-Host $_ }
                throw 'See above - amber-server status and amber-server logs tell more.'
            }
            Start-Sleep -Seconds 1
            $waited++
        }
    } else {
        Write-Host ''
        Write-Host "The background task could not be set up - start the server yourself: $bin run"
    }
    Write-Host ''
    & $bin url
    Write-Host ''
    Write-Host 'Optional - tidier text with a local language model (a large download): amber-server cleanup add'
}

try {
    Install-AmberServer
} catch {
    Write-Host "amber-server install: $($_.Exception.Message)" -ForegroundColor Red
}
