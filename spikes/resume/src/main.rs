use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use qs_resume_spike::{CrashPoint, ResumeReceiver};
use std::{path::PathBuf, thread, time::Duration};
use tempfile::tempdir;

#[derive(Debug, Parser)]
#[command(about = "Disposable Quick Share resume/crash-consistency experiment")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run an in-process injected-failure demonstration.
    Demo {
        #[arg(long, default_value_t = 32)]
        size_mib: usize,
        #[arg(long, default_value_t = 4)]
        chunk_mib: usize,
    },
    /// Persist work under --root so the process can be killed and restarted.
    Worker {
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value_t = 256)]
        size_mib: usize,
        #[arg(long, default_value_t = 4)]
        chunk_mib: usize,
        #[arg(long, default_value_t = 100)]
        delay_ms: u64,
    },
}

fn generated_chunk(index: usize, chunk_size: usize, total_size: usize) -> Vec<u8> {
    let start = index * chunk_size;
    let end = (start + chunk_size).min(total_size);
    (start..end).map(|offset| (offset % 251) as u8).collect()
}

fn generated_digest(size: usize, chunk_size: usize) -> String {
    let mut hasher = blake3::Hasher::new();
    for index in 0..size.div_ceil(chunk_size) {
        hasher.update(&generated_chunk(index, chunk_size, size));
    }
    hasher.finalize().to_hex().to_string()
}

fn demo(size_mib: usize, chunk_mib: usize) -> Result<()> {
    ensure!(chunk_mib > 0, "chunk size must be positive");
    let size = size_mib * 1024 * 1024;
    let chunk_size = chunk_mib * 1024 * 1024;
    let payload: Vec<u8> = (0..size).map(|offset| (offset % 251) as u8).collect();
    let final_digest = blake3::hash(&payload).to_hex().to_string();
    let temp = tempdir().context("create experiment directory")?;
    let mut receiver =
        ResumeReceiver::create(temp.path(), "final.bin", size as u64, chunk_size as u32)?;

    let first = &payload[..chunk_size.min(payload.len())];
    let first_digest = blake3::hash(first).to_hex();
    let injected = receiver.write_chunk(0, first, first_digest.as_ref(), CrashPoint::AfterDataSync);
    println!("Injected failure: {injected:?}");
    drop(receiver);

    let mut receiver = ResumeReceiver::reopen(temp.path(), "final.bin")?;
    println!("Missing after reopen: {:?}", receiver.missing_chunks());
    for index in receiver.missing_chunks() {
        let start = index * chunk_size;
        let end = (start + chunk_size).min(payload.len());
        let bytes = &payload[start..end];
        let chunk_digest = blake3::hash(bytes).to_hex();
        receiver.write_chunk(index, bytes, chunk_digest.as_ref(), CrashPoint::None)?;
    }
    let final_path = receiver.finalize(&final_digest)?;
    ensure!(
        std::fs::read(&final_path)? == payload,
        "final payload mismatch"
    );
    println!("PASS: resumed and verified {} bytes", size);
    Ok(())
}

fn worker(root: PathBuf, size_mib: usize, chunk_mib: usize, delay_ms: u64) -> Result<()> {
    ensure!(chunk_mib > 0, "chunk size must be positive");
    let size = size_mib * 1024 * 1024;
    let chunk_size = chunk_mib * 1024 * 1024;
    ensure!(chunk_size <= u32::MAX as usize, "chunk size exceeds u32");
    std::fs::create_dir_all(&root)?;
    println!("ROOT {}", root.display());

    let mut receiver = if root.join("state.json").exists() {
        println!("OPEN existing journal");
        ResumeReceiver::reopen(&root, "final.bin")?
    } else {
        println!("CREATE new journal");
        ResumeReceiver::create(&root, "final.bin", size as u64, chunk_size as u32)?
    };
    let missing = receiver.missing_chunks();
    println!("MISSING {} chunks: {missing:?}", missing.len());

    for index in missing {
        let bytes = generated_chunk(index, chunk_size, size);
        let digest = blake3::hash(&bytes).to_hex();
        receiver.write_chunk(index, &bytes, digest.as_ref(), CrashPoint::None)?;
        println!("COMMITTED chunk {index}");
        thread::sleep(Duration::from_millis(delay_ms));
    }

    let final_digest = generated_digest(size, chunk_size);
    let path = receiver.finalize(&final_digest)?;
    println!("PASS final={} bytes={size}", path.display());
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Demo {
            size_mib,
            chunk_mib,
        } => demo(size_mib, chunk_mib),
        Command::Worker {
            root,
            size_mib,
            chunk_mib,
            delay_ms,
        } => worker(root, size_mib, chunk_mib, delay_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::{generated_chunk, generated_digest};

    #[test]
    fn generated_chunks_have_same_digest_as_contiguous_payload() {
        // Arrange
        let payload: Vec<u8> = (0..25).map(|offset| (offset % 251) as u8).collect();

        // Act
        let chunks: Vec<u8> = (0..4)
            .flat_map(|index| generated_chunk(index, 8, payload.len()))
            .collect();
        let digest = generated_digest(payload.len(), 8);

        // Assert
        assert_eq!(chunks, payload);
        assert_eq!(digest, blake3::hash(&payload).to_hex().to_string());
    }
}
