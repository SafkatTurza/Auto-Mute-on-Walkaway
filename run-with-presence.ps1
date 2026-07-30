<#
.SYNOPSIS
    Launch Auto-Mute on Walkaway with webcam presence detection wired up.

.DESCRIPTION
    The app defaults to calling `python3` for the presence sidecar, but on
    Windows the interpreter that actually has MediaPipe + OpenCV installed is
    usually a specific version (3.9-3.12 — MediaPipe has no wheels for 3.13+).
    This script finds that interpreter, verifies the detector's dependencies,
    points the app at it via the AMOW_PRESENCE_* environment variables, and
    starts the app — so you never have to set those variables by hand.

    Everything runs locally; no camera frames, audio, or data leave the machine.

.PARAMETER Dev
    Run `npm run tauri dev` from source instead of launching an installed build.

.PARAMETER AsAdmin
    Relaunch elevated. Camera *disable* needs administrator rights; mic mute and
    presence detection do not. Without this the app still runs and mutes the mic,
    it just leaves the camera untouched.

.PARAMETER Install
    Install the detector's Python dependencies
    (`pip install -e "presence-detector[camera]"`) before starting.

.EXAMPLE
    .\run-with-presence.ps1
    Launch the installed app with presence detection.

.EXAMPLE
    .\run-with-presence.ps1 -AsAdmin
    Same, elevated, so the camera can be disabled on walkaway.

.EXAMPLE
    .\run-with-presence.ps1 -Dev
    Run from source with `npm run tauri dev`.
#>
[CmdletBinding()]
param(
    [switch]$Dev,
    [switch]$AsAdmin,
    [switch]$Install
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$DetectorDir = Join-Path $RepoRoot "presence-detector"

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "    $msg" -ForegroundColor Green }
function Write-Warn2($msg){ Write-Host "    $msg" -ForegroundColor Yellow }

# --- Optional self-elevation --------------------------------------------------
# Re-launch the *same* script with the same switches under an elevated shell.
if ($AsAdmin) {
    $isAdmin = ([Security.Principal.WindowsPrincipal] `
        [Security.Principal.WindowsIdentity]::GetCurrent()
    ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

    if (-not $isAdmin) {
        Write-Step "Elevating (camera control needs administrator rights)..."
        $fwd = @("-NoProfile", "-ExecutionPolicy", "Bypass",
                 "-File", "`"$($MyInvocation.MyCommand.Path)`"")
        if ($Dev)     { $fwd += "-Dev" }
        if ($Install) { $fwd += "-Install" }
        Start-Process -FilePath "powershell.exe" -Verb RunAs -ArgumentList $fwd
        return
    }
    Write-Ok "Running as administrator."
}

# --- 1. Locate a MediaPipe-capable Python -------------------------------------
Write-Step "Locating a Python interpreter for the presence sidecar..."

# Candidates, best first: an explicit override, then the py launcher pinned to
# versions MediaPipe ships wheels for, then whatever `python` resolves to.
$candidates = @()
if ($env:AMOW_PRESENCE_PYTHON) { $candidates += ,@($env:AMOW_PRESENCE_PYTHON) }
$candidates += ,@("py", "-3.12")
$candidates += ,@("py", "-3.11")
$candidates += ,@("py", "-3.10")
$candidates += ,@("py", "-3.9")
$candidates += ,@("python")

$python = $null
foreach ($c in $candidates) {
    $exe    = $c[0]
    $pyArgs = if ($c.Count -gt 1) { $c[1..($c.Count - 1)] } else { @() }
    try {
        # Resolve to a concrete interpreter path so the app (which does not know
        # about the `py` launcher) can invoke it directly.
        $resolved = & $exe @pyArgs -c "import sys; print(sys.executable)" 2>$null
        if ($LASTEXITCODE -eq 0 -and $resolved) {
            $ver = & $resolved -c "import sys; print('%d.%d' % sys.version_info[:2])" 2>$null
            $python = $resolved.Trim()
            Write-Ok "Using Python $ver at $python"
            break
        }
    } catch { }
}

if (-not $python) {
    Write-Warn2 "No Python interpreter found."
    Write-Warn2 "Install Python 3.12:  winget install --id Python.Python.3.12"
    Write-Warn2 "The app will still run and mute the mic; presence falls back to the manual Away toggle."
    $python = $null
}

# --- 2. Verify / install detector dependencies --------------------------------
if ($python) {
    if ($Install) {
        Write-Step "Installing detector dependencies..."
        & $python -m pip install -e "$DetectorDir[camera]"
    }

    Write-Step "Checking the detector's dependencies..."
    & $python -c "import cv2, mediapipe" 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Warn2 "OpenCV / MediaPipe are not importable in this interpreter."
        Write-Warn2 "Install them with:"
        Write-Warn2 "    & `"$python`" -m pip install -e `"$DetectorDir[camera]`""
        Write-Warn2 "Or re-run this script with -Install. Continuing without presence for now."
        $python = $null
    } else {
        Write-Ok "OpenCV and MediaPipe are available."
    }
}

# --- 3. Point the app at the sidecar ------------------------------------------
if ($python) {
    $env:AMOW_PRESENCE_PYTHON = $python
    $env:AMOW_PRESENCE_DIR    = $DetectorDir
    Remove-Item Env:\AMOW_PRESENCE_DISABLE -ErrorAction SilentlyContinue
    Write-Ok "AMOW_PRESENCE_PYTHON = $python"
    Write-Ok "AMOW_PRESENCE_DIR    = $DetectorDir"
} else {
    # Be explicit rather than let the app try (and fail to find) `python3`.
    $env:AMOW_PRESENCE_DISABLE = "1"
    Write-Warn2 "Presence sidecar disabled; using the manual Away toggle."
}

# --- 4. Launch ----------------------------------------------------------------
if ($Dev) {
    Write-Step "Starting the app from source (npm run tauri dev)..."
    Push-Location $RepoRoot
    try { npm run tauri dev } finally { Pop-Location }
    return
}

Write-Step "Locating an installed / built app executable..."
$exeName   = "Auto-Mute on Walkaway.exe"
$appExe    = $null
$searchDirs = @(
    (Join-Path $env:LOCALAPPDATA "Auto-Mute on Walkaway"),
    (Join-Path $env:LOCALAPPDATA "Programs\Auto-Mute on Walkaway"),
    (Join-Path $RepoRoot "src-tauri\target\release"),
    (Join-Path $RepoRoot "src-tauri\target\debug")
)
foreach ($d in $searchDirs) {
    $candidate = Join-Path $d $exeName
    if (Test-Path $candidate) { $appExe = $candidate; break }
}

if ($appExe) {
    Write-Ok "Launching $appExe"
    # Start in a way that keeps the AMOW_PRESENCE_* env we just set.
    & $appExe
} else {
    Write-Warn2 "No installed or built executable found."
    Write-Warn2 "Build one with:  npm run tauri build"
    Write-Warn2 "Or run from source with:  .\run-with-presence.ps1 -Dev"
    exit 1
}
