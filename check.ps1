# Abbey Bot Windows gate. Keep this behavior aligned with check.sh, except for
# POSIX shell/plist checks which cannot run meaningfully on a Windows host.
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
Set-Location $PSScriptRoot

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)] [string] $Executable,
        [Parameter(Mandatory = $true)] [string[]] $Arguments
    )

    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Executable failed with exit code $LASTEXITCODE"
    }
}

Write-Host "== fmt =="
Invoke-Checked -Executable "cargo" -Arguments @("fmt", "--all", "--", "--check")

Write-Host "== deploy and privacy validation =="
Invoke-Checked -Executable "python" -Arguments @("scripts/check-python-syntax.py")
Invoke-Checked -Executable "python" -Arguments @(
    "deploy/check-python-locks.py",
    "deploy/mlx-vlm-requirements.txt",
    "deploy/mlx-audio-requirements.txt",
    "deploy/mlx-audio-build-constraints.txt"
)
Invoke-Checked -Executable "python" -Arguments @("deploy/test-configure-mlx-primary.py")
Invoke-Checked -Executable "python" -Arguments @("deploy/test-publish-provider-qualification.py")
Invoke-Checked -Executable "python" -Arguments @("deploy/test-smoke-mlx-vlm-tool-deltas.py")
Invoke-Checked -Executable "python" -Arguments @("deploy/test-patch-mlx-vlm-tool-encoding.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/check-privacy.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/test-check-pages-liquid.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/check-pages-liquid.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/test-check-abbey-contracts.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/check-abbey-contracts.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/test-check-linux-tls-tree.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/check-linux-tls-tree.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/test-check-rustsec-debt.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/check-rustsec-debt.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/test-check-wdbx-conformance.py")
Invoke-Checked -Executable "python" -Arguments @("scripts/check-wdbx-conformance.py")

Write-Host "audio-tap installer and Swift gate: skipped (requires POSIX/macOS); Python syntax checked above"

Write-Host "== clippy =="
Invoke-Checked -Executable "cargo" -Arguments @("clippy", "--all-targets", "--locked", "--", "-D", "warnings")

Write-Host "== test =="
Invoke-Checked -Executable "cargo" -Arguments @("test", "--locked")

Write-Host "== release build =="
Invoke-Checked -Executable "cargo" -Arguments @("build", "--release", "--locked")

Write-Host "== ok =="
