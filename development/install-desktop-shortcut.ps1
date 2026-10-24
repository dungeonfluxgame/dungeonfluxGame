[CmdletBinding(SupportsShouldProcess)]
param(
    [string]$RepositoryRoot = (Split-Path -Parent $PSScriptRoot),
    [string]$BuildRoot,
    [string]$PythonPath = 'C:\Python311\python.exe'
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
if (-not $BuildRoot) {
    $BuildRoot = Join-Path $repository 'artifacts/build/launch-refinement'
}
$build = (Resolve-Path -LiteralPath $BuildRoot).Path
$python = (Resolve-Path -LiteralPath $PythonPath).Path
$launcher = Join-Path $repository 'development/dev-gameplay.py'
$binary = Join-Path $build 'debug/df-transport-fixture.exe'
$icon = Join-Path $repository 'assets/ui/app/dungeonflux.ico'
foreach ($required in @($python, $launcher, $binary, $icon, (Join-Path $build 'play-web/df_tools.js'), (Join-Path $build 'play-web/df_tools_bg.wasm'))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required file is missing: $required"
    }
}
$desktop = [Environment]::GetFolderPath('DesktopDirectory')
if (-not $desktop) {
    throw 'Windows Desktop directory is unavailable.'
}
$shortcutPath = Join-Path $desktop 'DungeonFlux (Development).lnk'
$description = 'DungeonFlux development: character creation'
$arguments = '"{0}" --screen creation --role player1 --no-build --build-root "{1}"' -f $launcher, $build
$shell = New-Object -ComObject WScript.Shell
if (Test-Path -LiteralPath $shortcutPath) {
    $existing = $shell.CreateShortcut($shortcutPath)
    if ($existing.Description -ne $description) {
        throw "An unrelated shortcut already exists at $shortcutPath; choose a different name or move it manually."
    }
}
if ($PSCmdlet.ShouldProcess($shortcutPath, 'Create or update DungeonFlux development shortcut')) {
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $python
    $shortcut.Arguments = $arguments
    $shortcut.WorkingDirectory = $repository
    $shortcut.IconLocation = "$icon,0"
    $shortcut.Description = $description
    $shortcut.WindowStyle = 1
    $shortcut.Save()
    Write-Output "Installed: $shortcutPath"
}

