# Builds the release program and zips it with its README into dist\.
# Run from the repository root:  powershell -File scripts\package-windows.ps1
$ErrorActionPreference = 'Stop'
# Keep the build machine's user name and folders out of the program (panic locations, PDB path).
$home_dir = $env:USERPROFILE
$src_dir = (Get-Location).Path
$env:RUSTFLAGS = "--remap-path-prefix=$home_dir=C:\user --remap-path-prefix=$src_dir=C:\src -C link-arg=/PDBALTPATH:pasteroll.pdb"
cargo build --release -p pasteroll-win
$out = 'dist\pasteroll-windows-x64'
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item 'target\release\pasteroll.exe' "$out\pasteroll.exe"
Copy-Item 'packaging\README-windows.txt' "$out\README.txt"
Copy-Item 'LICENSE-MIT', 'LICENSE-APACHE' $out
Compress-Archive -Path "$out\*" -DestinationPath 'dist\pasteroll-windows-x64.zip' -Force
certutil -hashfile 'dist\pasteroll-windows-x64.zip' SHA256
