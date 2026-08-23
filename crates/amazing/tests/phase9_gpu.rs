use std::process::Command;

#[test]
#[cfg(windows)]
#[ignore = "requires a real D3D11 hardware adapter; run explicitly on win-001"]
fn gpu_info_reports_a_real_d3d11_hardware_device() {
    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("gpu-info")
        .output()
        .expect("run GPU capability artifact");
    assert!(
        result.status.success(),
        "gpu-info failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("UTF-8 command output");
    assert!(stdout.contains("D3D11 hardware device"), "{stdout}");
    assert!(
        stdout.contains("feature level 11") || stdout.contains("feature level 10"),
        "{stdout}"
    );
}
