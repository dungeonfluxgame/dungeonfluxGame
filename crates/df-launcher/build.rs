//! Embeds the DungeonFlux application icon in the Windows launcher executable, the same way
//! `crates/df-tools/build.rs` embeds it in the game server. Other targets skip this step.
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn resource_compiler() -> io::Result<PathBuf> {
    if let Some(compiler) = env::var_os("RC") {
        return Ok(PathBuf::from(compiler));
    }
    if let Some(paths) = env::var_os("PATH") {
        for directory in env::split_paths(&paths) {
            let compiler = directory.join("rc.exe");
            if compiler.is_file() {
                return Ok(compiler);
            }
        }
    }
    let sdk_root = env::var_os("WindowsSdkDir")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("ProgramFiles(x86)").map(|root| PathBuf::from(root).join("Windows Kits/10"))
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Windows SDK not found; install Windows SDK or set RC to rc.exe",
            )
        })?;
    let mut versions = std::fs::read_dir(sdk_root.join("bin"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    versions.sort();
    let host = env::var("HOST").unwrap_or_default();
    let architectures = if host.starts_with("aarch64") {
        ["arm64", "x64", "x86"]
    } else {
        ["x64", "x86", "arm64"]
    };
    for version in versions.iter().rev() {
        for architecture in architectures {
            let compiler = version.join(architecture).join("rc.exe");
            if compiler.is_file() {
                return Ok(compiler);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "rc.exe not found in Windows SDK; install Windows SDK or set RC",
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=windows/dungeonflux.rc");
    println!("cargo:rerun-if-changed=../../assets/ui/app/dungeonflux.ico");
    for variable in ["RC", "PATH", "WindowsSdkDir", "ProgramFiles(x86)"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let target = env::var("TARGET")?;
    let host = env::var("HOST")?;
    if !target.contains("windows-msvc") || !host.contains("windows-msvc") {
        return Ok(());
    }
    let manifest_directory =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing CARGO_MANIFEST_DIR")?);
    let resources =
        PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?).join("dungeonflux.res");
    let compiler = resource_compiler()?;
    let status = Command::new(&compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resources)
        .arg("/I")
        .arg(manifest_directory.join(Path::new("../../assets/ui/app")))
        .arg(manifest_directory.join("windows/dungeonflux.rc"))
        .status()?;
    if !status.success() {
        return Err(format!(
            "Windows resource compiler {} failed: {status}",
            compiler.display()
        )
        .into());
    }
    println!(
        "cargo:rustc-link-arg-bin=dungeonflux={}",
        resources.display()
    );
    Ok(())
}
