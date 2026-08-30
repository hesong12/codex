//! Host-requested process isolation for model-reachable child processes.
//!
//! The ordinary filesystem/network sandbox describes what a child may do to
//! user resources. This guard is a separate, monotonic boundary that protects
//! the embedding host and its credential brokers even when the ordinary
//! sandbox posture is full access.

use std::ffi::OsStr;
use std::io;

/// Launch-only setting supplied by an embedding host.
///
/// The value is deliberately non-secret, but model-reachable children must not
/// inherit it because it describes authority held by their parent process.
pub const HOST_SECRET_GUARD_ENV_VAR: &str = "CODEX_HOST_SECRET_GUARD";

const REQUIRED_VALUE: &str = "required";

#[cfg(target_os = "macos")]
const MACOS_HOST_SECRET_POLICY: &str = r#"(version 1)
(allow default)
(deny process-info*)
"#;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HostSecretGuardRequirement {
    #[default]
    Disabled,
    Required,
    Invalid,
}

impl HostSecretGuardRequirement {
    pub fn from_process_environment() -> Self {
        match std::env::var(HOST_SECRET_GUARD_ENV_VAR) {
            Err(std::env::VarError::NotPresent) => Self::Disabled,
            Ok(value) if value == REQUIRED_VALUE => Self::Required,
            Ok(_) | Err(std::env::VarError::NotUnicode(_)) => Self::Invalid,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostSecretGuardAttestation {
    pub requested: bool,
    pub enforced: bool,
    pub backend: &'static str,
    pub inherited_handle_policy: &'static str,
}

pub fn host_secret_guard_attestation() -> HostSecretGuardAttestation {
    host_secret_guard_attestation_for(HostSecretGuardRequirement::from_process_environment())
}

pub fn host_secret_guard_required() -> bool {
    HostSecretGuardRequirement::from_process_environment() == HostSecretGuardRequirement::Required
}

pub fn host_secret_guard_attestation_for(
    requirement: HostSecretGuardRequirement,
) -> HostSecretGuardAttestation {
    match requirement {
        HostSecretGuardRequirement::Disabled => HostSecretGuardAttestation {
            requested: false,
            enforced: false,
            backend: "disabled",
            inherited_handle_policy: inherited_handle_policy(),
        },
        HostSecretGuardRequirement::Invalid => HostSecretGuardAttestation {
            requested: true,
            enforced: false,
            backend: "invalid-configuration",
            inherited_handle_policy: inherited_handle_policy(),
        },
        HostSecretGuardRequirement::Required => required_attestation(),
    }
}

pub fn model_child_tokio_command(
    program: impl AsRef<OsStr>,
) -> io::Result<tokio::process::Command> {
    model_child_tokio_command_for(
        program,
        HostSecretGuardRequirement::from_process_environment(),
    )
}

pub fn model_child_tokio_command_for(
    program: impl AsRef<OsStr>,
    requirement: HostSecretGuardRequirement,
) -> io::Result<tokio::process::Command> {
    ensure_enforced(requirement)?;
    #[cfg(target_os = "macos")]
    if requirement == HostSecretGuardRequirement::Required {
        let mut command = tokio::process::Command::new("/usr/bin/sandbox-exec");
        command.args(["-p", MACOS_HOST_SECRET_POLICY]).arg(program);
        return Ok(command);
    }
    Ok(tokio::process::Command::new(program))
}

#[cfg(unix)]
pub fn model_child_std_command(program: impl AsRef<OsStr>) -> io::Result<std::process::Command> {
    model_child_std_command_for(
        program,
        HostSecretGuardRequirement::from_process_environment(),
    )
}

#[cfg(unix)]
pub fn model_child_std_command_for(
    program: impl AsRef<OsStr>,
    requirement: HostSecretGuardRequirement,
) -> io::Result<std::process::Command> {
    ensure_enforced(requirement)?;
    #[cfg(target_os = "macos")]
    if requirement == HostSecretGuardRequirement::Required {
        let mut command = std::process::Command::new("/usr/bin/sandbox-exec");
        command.args(["-p", MACOS_HOST_SECRET_POLICY]).arg(program);
        return Ok(command);
    }
    Ok(std::process::Command::new(program))
}

/// Close every inherited Unix descriptor except the caller's explicit list.
///
/// Windows guarded launches fail before this point until an implementation can
/// provide a real `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` for full-access children.
pub fn apply_inherited_handle_allowlist(
    command: &mut tokio::process::Command,
    preserved_fds: &[i32],
) -> io::Result<()> {
    ensure_enforced(HostSecretGuardRequirement::from_process_environment())?;
    #[cfg(unix)]
    {
        let preserved_fds = preserved_fds.to_vec();
        unsafe {
            command.pre_exec(move || {
                crate::pty::close_inherited_fds_except(&preserved_fds);
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = preserved_fds;
    Ok(())
}

fn ensure_enforced(requirement: HostSecretGuardRequirement) -> io::Result<()> {
    let attestation = host_secret_guard_attestation_for(requirement);
    if !attestation.requested || attestation.enforced {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "host secret guard was required but backend `{}` cannot enforce it",
            attestation.backend
        ),
    ))
}

#[cfg(target_os = "macos")]
fn required_attestation() -> HostSecretGuardAttestation {
    HostSecretGuardAttestation {
        requested: true,
        enforced: true,
        backend: "macos-seatbelt-process-isolation",
        inherited_handle_policy: "explicit-fd-allowlist",
    }
}

#[cfg(target_os = "windows")]
fn required_attestation() -> HostSecretGuardAttestation {
    HostSecretGuardAttestation {
        requested: true,
        enforced: false,
        backend: "windows-full-access-unavailable",
        inherited_handle_policy: "unavailable",
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn required_attestation() -> HostSecretGuardAttestation {
    HostSecretGuardAttestation {
        requested: true,
        enforced: false,
        backend: "unsupported-platform",
        inherited_handle_policy: inherited_handle_policy(),
    }
}

#[cfg(unix)]
fn inherited_handle_policy() -> &'static str {
    "explicit-fd-allowlist"
}

#[cfg(not(unix))]
fn inherited_handle_policy() -> &'static str {
    "platform-default"
}

#[cfg(test)]
#[path = "host_secret_guard_tests.rs"]
mod tests;
