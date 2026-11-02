$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    cargo install cargo-packager --version 0.11.8 --locked
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

cargo packager --release --packages whelper
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
