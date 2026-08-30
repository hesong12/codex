use super::*;
use pretty_assertions::assert_eq;

#[test]
fn disabled_attestation_does_not_claim_enforcement() {
    assert_eq!(
        host_secret_guard_attestation_for(HostSecretGuardRequirement::Disabled),
        HostSecretGuardAttestation {
            requested: false,
            enforced: false,
            backend: "disabled",
            inherited_handle_policy: inherited_handle_policy(),
        }
    );
}

#[test]
fn invalid_configuration_fails_closed() {
    let error = model_child_tokio_command_for("ignored", HostSecretGuardRequirement::Invalid)
        .expect_err("invalid guard configuration must reject child launch");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_required_guard_blocks_cross_process_inspection_but_keeps_file_access() {
    let guard_file = std::env::temp_dir().join(format!(
        "codex-host-secret-guard-test-{}",
        std::process::id()
    ));
    let output = model_child_std_command_for("/bin/sh", HostSecretGuardRequirement::Required)
        .expect("build guarded command")
        .args([
            "-c",
            "printf guarded > \"$1\" && /bin/cat \"$1\"; /bin/ps -p 1",
            "host-secret-guard-test",
        ])
        .arg(&guard_file)
        .output()
        .expect("run guarded command");
    let _ = std::fs::remove_file(guard_file);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("guarded"));
}

#[cfg(target_os = "windows")]
#[test]
fn windows_required_guard_refuses_unattested_full_access_launch() {
    let attestation = host_secret_guard_attestation_for(HostSecretGuardRequirement::Required);
    assert!(!attestation.enforced);
    assert_eq!(attestation.backend, "windows-full-access-unavailable");
}
