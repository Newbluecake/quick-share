use crate::{ExecutableReplacer, UpdateError};
use async_trait::async_trait;
use semver::Version;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Debug, Default)]
pub struct SafeSelfReplacer;

#[async_trait]
impl ExecutableReplacer for SafeSelfReplacer {
    async fn replace(
        &self,
        current_executable: &Path,
        candidate: &Path,
        expected_version: &Version,
    ) -> Result<(), UpdateError> {
        let actual_current = std::env::current_exe()?.canonicalize()?;
        let requested_current = current_executable.canonicalize()?;
        if actual_current != requested_current {
            return Err(UpdateError::ReplacementFailed(
                "replacement target is not the running executable".to_owned(),
            ));
        }
        verify_candidate(candidate, expected_version).await?;
        let backup = backup_path(current_executable)?;
        copy_durable(current_executable, &backup)?;

        if let Err(error) = self_replace::self_replace(candidate) {
            rollback_to_backup(&backup, current_executable)?;
            return Err(UpdateError::ReplacementFailed(format!(
                "platform replacement failed and was rolled back ({error})"
            )));
        }

        if let Err(error) = verify_candidate(current_executable, expected_version).await {
            rollback_to_backup(&backup, current_executable)?;
            return Err(UpdateError::ReplacementFailed(format!(
                "new executable failed its startup check and was rolled back ({error})"
            )));
        }

        let _ = fs::remove_file(&backup);
        sync_parent(current_executable)?;
        Ok(())
    }
}

async fn verify_candidate(path: &Path, expected: &Version) -> Result<(), UpdateError> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| UpdateError::CandidateDidNotStart(expected.clone()))?
        .map_err(|_| UpdateError::CandidateDidNotStart(expected.clone()))?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return Err(UpdateError::CandidateDidNotStart(expected.clone()));
    }
    let version = String::from_utf8(output.stdout)
        .map_err(|_| UpdateError::CandidateDidNotStart(expected.clone()))?;
    if version.trim() != format!("quick-share {expected}") {
        return Err(UpdateError::CandidateDidNotStart(expected.clone()));
    }
    Ok(())
}

fn backup_path(current: &Path) -> Result<PathBuf, UpdateError> {
    let parent = current.parent().ok_or(UpdateError::ExecutableHasNoParent)?;
    let mut random = [0_u8; 12];
    getrandom::fill(&mut random).map_err(|_| UpdateError::Randomness)?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    Ok(parent.join(format!(
        ".quick-share-backup-{}{suffix}",
        hex::encode(random)
    )))
}

fn copy_durable(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.set_permissions(input.metadata()?.permissions())?;
    output.sync_all()?;
    sync_parent(destination)?;
    Ok(())
}

fn rollback_to_backup(backup: &Path, destination: &Path) -> Result<(), UpdateError> {
    if destination.exists() {
        if same_contents(backup, destination).unwrap_or(false) {
            fs::remove_file(backup)?;
            sync_parent(destination)?;
            return Ok(());
        }
        let broken = backup_path(destination)?;
        fs::rename(destination, &broken).map_err(|error| {
            UpdateError::RollbackFailed(format!(
                "could not move failed candidate aside: {error}; backup is {}",
                backup.display()
            ))
        })?;
        if let Err(error) = restore_backup(backup, destination) {
            let _ = fs::rename(&broken, destination);
            return Err(error);
        }
        let _ = fs::remove_file(broken);
        return Ok(());
    }
    restore_backup(backup, destination)
}

fn same_contents(left: &Path, right: &Path) -> Result<bool, std::io::Error> {
    use std::io::Read;

    if fs::metadata(left)?.len() != fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left = fs::File::open(left)?;
    let mut right = fs::File::open(right)?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_count = left.read(&mut left_buffer)?;
        let right_count = right.read(&mut right_buffer)?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

fn restore_backup(backup: &Path, destination: &Path) -> Result<(), UpdateError> {
    fs::rename(backup, destination).map_err(|error| {
        UpdateError::RollbackFailed(format!(
            "runnable backup remains at {}: {error}",
            backup.display()
        ))
    })?;
    sync_parent(destination)?;
    Ok(())
}

fn sync_parent(path: &Path) -> Result<(), UpdateError> {
    #[cfg(unix)]
    {
        let parent = path.parent().ok_or(UpdateError::ExecutableHasNoParent)?;
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
