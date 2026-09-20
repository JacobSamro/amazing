//! Default Windows application launch policy.
use std::process::{Command, ExitCode, ExitStatus};
use crate::gui;
#[cfg(windows)]
pub(crate) fn default_gpui_shell() -> ExitCode {
    let current_exe = std::env::current_exe().ok();
    if let Some(current_exe) = current_exe.as_ref()
        && let Some(binary_dir) = current_exe.parent()
    {
        let mut candidates = vec![binary_dir.join("amazing-shell.exe")];
        if let Some(target_dir) = binary_dir.parent() {
            candidates.push(target_dir.join("release").join("amazing-shell.exe"));
            candidates.push(target_dir.join("debug").join("amazing-shell.exe"));
        }
        for candidate in candidates {
            if candidate == *current_exe || !candidate.is_file() {
                continue;
            }
            match Command::new(&candidate).status() {
                Ok(status) => return process_exit_code(status),
                Err(error) => eprintln!(
                    "amazing: could not start GPUI shell {}: {error}",
                    candidate.display()
                ),
            }
        }
    }

    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent);
    if let Some(workspace) = workspace {
        let manifest = workspace.join("crates/amazing-shell-gpui/Cargo.toml");
        if manifest.is_file() {
            eprintln!("amazing: building the GPUI application shell…");
            match Command::new("cargo")
                .current_dir(workspace)
                .arg("run")
                .arg("--manifest-path")
                .arg(&manifest)
                .arg("--target-dir")
                .arg(workspace.join("target"))
                .arg("--locked")
                .status()
            {
                Ok(status) => return process_exit_code(status),
                Err(error) => eprintln!("amazing: GPUI shell launch failed: {error}"),
            }
        }
    }

    eprintln!("amazing: GPUI shell unavailable; starting the Win32 fallback");
    gui::run(None)
}

#[cfg(windows)]
fn process_exit_code(status: ExitStatus) -> ExitCode {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from)
}

