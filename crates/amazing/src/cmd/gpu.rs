//! GPU capability and presentation commands.
use std::process::ExitCode;
use crate::gpu;
/// Phase 9.4 artifact: require a real Direct3D 11 hardware device.
pub(crate) fn cmd_gpu_info() -> ExitCode {
    match gpu::probe() {
        Ok(info) => {
            println!("{}, feature level {}", info.backend, info.feature_level);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing gpu-info: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.5 artifact: upload and present a BGRA surface through DXGI.
pub(crate) fn cmd_gpu_present_smoke() -> ExitCode {
    match gpu::presentation_smoke() {
        Ok(info) => {
            println!(
                "{}, presented {}x{} BGRA",
                info.backend, info.width, info.height
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing gpu-present-smoke: {error}");
            ExitCode::FAILURE
        }
    }
}

