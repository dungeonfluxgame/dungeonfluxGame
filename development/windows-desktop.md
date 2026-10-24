# Windows desktop entry and application icon

The current executable is a development server, not the planned graphical
account/sign-in launcher. Double-clicking the development shortcut starts the
Python development launcher in a visible console, then opens the character
creation browser screen. Stop this development session with Ctrl-C in its console.
The product behavior where closing a launcher leaves the game running is still
planned and is not implemented by this shortcut.

## Install the shortcut

First build and verify the development game using the launch guide. From the
repository root in PowerShell:

```powershell
./development/install-desktop-shortcut.ps1 -WhatIf
./development/install-desktop-shortcut.ps1
```

This creates **DungeonFlux (Development)** on the current user's Windows Desktop
(including a redirected Desktop). It uses `assets/ui/app/dungeonflux.ico` and
starts this equivalent command:

```powershell
C:/Python311/python.exe development/dev-gameplay.py --screen creation --role player1 --no-build --build-root artifacts/build/launch-refinement
```

The installed shortcut stores absolute paths and uses the repository as its
working directory. Moving the repository requires reinstalling the shortcut.

Installer options:

| Option | Meaning |
| --- | --- |
| `-RepositoryRoot <path>` | Repository containing the launcher and application icon; defaults to this script's parent repository. |
| `-BuildRoot <path>` | Existing verified Cargo build root; defaults to `artifacts/build/launch-refinement`. Requires `debug/df-transport-fixture.exe` plus `play-web/df_tools.js` and `play-web/df_tools_bg.wasm`. |
| `-PythonPath <path>` | Python executable; defaults to `C:/Python311/python.exe`. |
| `-WhatIf` | Validate required paths and report the proposed shortcut without saving it. |

The installer does not build, start the game, modify firewall settings or install
Python. It refuses to overwrite an existing shortcut with an unrelated description.
`--no-build` deliberately reuses the verified binary and browser assets; rebuild
when source changes. The shortcut's flags select the character-creation development
entry as player one; this is not account authentication. Consult the main launch
configuration guide for available screens, configuration precedence and executable
flags. The shortcut does not configure private provider credentials.

## Embedded Windows executable icon

Cargo automatically runs `crates/df-tools/build.rs`. For Windows MSVC builds on a
Windows MSVC host, it compiles `windows/dungeonflux.rc` with the Windows SDK resource
compiler and links the result into **df-transport-fixture.exe only**. No manifest
edit or additional Rust dependency is required. Non-Windows and WASM builds skip
this resource step.

The compiler is located via `RC`, then `PATH`, then the installed Windows SDK's
versioned `bin` directories. On ARM64 hosts the SDK ARM64 compiler is preferred.
Set `RC` to an explicit `rc.exe` path if needed. Otherwise install the Windows SDK
through Visual Studio Build Tools. The build fails with a resource-compiler error
if it cannot compile the icon; it does not silently produce an iconless Windows
binary. This is build-time configuration, not an executable runtime flag.

The resource contains only the application icon. Product version and publisher
metadata have not been invented. Icon source and generation provenance are in
`assets/ui/app/README.md`.

