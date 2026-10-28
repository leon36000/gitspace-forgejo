//! Local, non-production harness for the MISSION-012 agent-runtime exercise.

#[allow(dead_code)]
#[path = "../mission_012_host.rs"]
mod host;

use std::{
    env,
    fs::{self, DirBuilder},
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::ExitCode,
};

use serde_json::json;

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() != 8 || arguments[1] != "--verify-pair" {
        eprintln!(
            "usage: mission-012-verify-local --verify-pair BASE_DIR CANDIDATE_DIR NEW_PRIVATE_TARGET_DIR TRUSTED_GIT_REPOSITORY BASE_COMMIT_SHA CANDIDATE_COMMIT_SHA"
        );
        return ExitCode::from(64);
    }
    let base = PathBuf::from(&arguments[2]);
    let candidate = PathBuf::from(&arguments[3]);
    let target_root = PathBuf::from(&arguments[4]);
    let trusted_git_repository = PathBuf::from(&arguments[5]);
    let base_commit = &arguments[6];
    let candidate_commit = &arguments[7];
    if let Err(error) = DirBuilder::new().mode(0o700).create(&target_root) {
        eprintln!("cannot create the fresh private verifier target directory: {error}");
        return ExitCode::from(73);
    }
    let sysroot = match rust_sysroot() {
        Ok(sysroot) => sysroot,
        Err(error) => {
            eprintln!("cannot resolve the host Rust toolchain: {error}");
            return ExitCode::from(69);
        }
    };
    match host::verify_pair(
        &base,
        &candidate,
        &target_root,
        &sysroot,
        &trusted_git_repository,
        base_commit,
        candidate_commit,
    ) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
            );
            match report["verdict"].as_str() {
                Some("pass") => ExitCode::SUCCESS,
                Some("fail") => ExitCode::from(1),
                _ => ExitCode::from(2),
            }
        }
        Err(error) => {
            println!(
                "{}",
                json!({
                    "schema_version":1,
                    "verdict":"inconclusive",
                    "reason":error.code,
                    "action":error.action,
                    "details":error.details,
                })
            );
            ExitCode::from(2)
        }
    }
}

fn rust_sysroot() -> Result<PathBuf, String> {
    let path = env::var_os("PATH").ok_or_else(|| "host PATH is unavailable".to_owned())?;
    let output = std::process::Command::new("rustc")
        .env_clear()
        .env("PATH", path)
        .args(["--print", "sysroot"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("rustc exited with {}", output.status));
    }
    let value = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    let sysroot = PathBuf::from(value.trim());
    if !sysroot.is_absolute()
        || !fs::symlink_metadata(&sysroot).is_ok_and(|metadata| metadata.is_dir())
    {
        return Err("rustc returned a non-directory sysroot".into());
    }
    Ok(sysroot)
}
