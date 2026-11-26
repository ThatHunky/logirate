# Builds the release binaries and the per-user MSI into dist\.
# Needs: Rust (MSVC), .NET SDK, and WiX 5: dotnet tool install --global wix --version 5.0.2
#        wix extension add -g WixToolset.Util.wixext/5.0.2
# Not 'Stop': Windows PowerShell treats a native tool's stderr (cargo's progress) as an error.
# Failures are caught via $LASTEXITCODE instead.
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    $version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    New-Item -ItemType Directory -Force dist | Out-Null
    $msi = "dist\logirate-$version-x64.msi"

    wix build installer\logirate.wxs -arch x64 -ext WixToolset.Util.wixext `
        -d Version=$version -d BinDir=target\release -d SrcDir=. -o $msi
    if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }

    Get-FileHash $msi -Algorithm SHA256 | ForEach-Object { "$($_.Hash.ToLower())  $(Split-Path $msi -Leaf)" } |
        Set-Content -Encoding ascii "$msi.sha256"
    Get-Item $msi, "$msi.sha256" | Select-Object Name, Length
} finally {
    Pop-Location
}
