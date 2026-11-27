[CmdletBinding()]
param(
    [string] $Iscc = 'ISCC.exe'
)

$ErrorActionPreference = 'Stop'
$compiler = (Get-Command -Name $Iscc -CommandType Application).Source
Push-Location -LiteralPath $PSScriptRoot
try {
    $package = Get-FileHash -LiteralPath 'vendor/vac471lite.zip'
    if ($package.Hash -ine '1ed42a7e763bf6237438fb62191ae88b2922f014b37658b29d7801d72c9d5357') {
        throw 'The original VAC Lite 4.71 package is required to build the installer.'
    }
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Cargo failed with exit code $LASTEXITCODE." }
    & $compiler 'installer.iss'
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE." }
    Get-Item -LiteralPath 'dist/MemeSoundboard-Setup-x64.exe'
} finally {
    Pop-Location
}
