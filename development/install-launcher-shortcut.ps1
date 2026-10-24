[CmdletBinding(SupportsShouldProcess)]
param(
    [string]$RepositoryRoot,
    [string]$BuildRoot
)

# Creates a "DungeonFlux" desktop shortcut that opens the desktop launcher (dungeonflux.exe).
# Build the launcher first:
#   cargo build --manifest-path crates/df-launcher/Cargo.toml --target-dir artifacts/build/launcher
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 leaves $PSScriptRoot empty in parameter defaults, so resolve it here.
if (-not $RepositoryRoot) {
    $RepositoryRoot = Split-Path -Parent $PSScriptRoot
}
$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
if (-not $BuildRoot) {
    $BuildRoot = Join-Path $repository 'artifacts/build/launcher'
}
$launcher = Join-Path (Resolve-Path -LiteralPath $BuildRoot).Path 'debug/dungeonflux.exe'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw "The launcher is not built yet: $launcher"
}
$desktop = [Environment]::GetFolderPath('DesktopDirectory')
if (-not $desktop) {
    throw 'Windows Desktop directory is unavailable.'
}
$shortcutPath = Join-Path $desktop 'DungeonFlux.lnk'
$description = 'DungeonFlux launcher: start, resume and stop games'
$shell = New-Object -ComObject WScript.Shell
if (Test-Path -LiteralPath $shortcutPath) {
    $existing = $shell.CreateShortcut($shortcutPath)
    if ($existing.Description -ne $description) {
        throw "An unrelated shortcut already exists at $shortcutPath; move it or rename it first."
    }
}
if ($PSCmdlet.ShouldProcess($shortcutPath, 'Create or update the DungeonFlux launcher shortcut')) {
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $launcher
    $shortcut.WorkingDirectory = $repository
    # The executable carries the application icon as resource 1.
    $shortcut.IconLocation = "$launcher,0"
    $shortcut.Description = $description
    $shortcut.WindowStyle = 1
    $shortcut.Save()
    Write-Output "Installed: $shortcutPath"
}
