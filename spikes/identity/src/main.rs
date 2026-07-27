use anyhow::{Context, Result, anyhow, ensure};
use clap::{Parser, Subcommand};
use qs_identity_spike::{
    StaticKeypair, build_initiator, build_responder, complete_xx, generate_static_keypair,
    mitm_sas_evidence, sas,
};
use snow::HandshakeState;
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_FRAME: usize = 65_535;

#[derive(Debug, Parser)]
#[command(about = "Disposable Quick Share Noise XX identity experiment")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Local {
        /// Fail unless a terminating relay produces different codes.
        #[arg(long)]
        check_mitm: bool,
    },
    Server {
        #[arg(long, default_value = "0.0.0.0:54440")]
        bind: SocketAddr,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        expect_remote: Option<String>,
    },
    Client {
        #[arg(long)]
        connect: SocketAddr,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        expect_remote: Option<String>,
    },
}

fn load_or_generate_keypair(path: &Path) -> Result<StaticKeypair> {
    if path.exists() {
        let bytes = fs::read(path).with_context(|| format!("read key file {}", path.display()))?;
        return serde_json::from_slice(&bytes).context("parse key file");
    }

    let keypair = generate_static_keypair()?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("key file has no parent"))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(&keypair)?;
    fs::write(path, bytes).with_context(|| format!("write key file {}", path.display()))?;
    set_private_permissions(path)?;
    Ok(keypair)
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= MAX_FRAME, "frame exceeds {MAX_FRAME} bytes");
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(length <= MAX_FRAME, "peer frame exceeds {MAX_FRAME} bytes");
    let mut bytes = vec![0_u8; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn write_handshake(state: &mut HandshakeState, stream: &mut TcpStream) -> Result<()> {
    let mut output = vec![0_u8; MAX_FRAME];
    let written = state.write_message(&[], &mut output)?;
    write_frame(stream, &output[..written])
}

fn read_handshake(state: &mut HandshakeState, stream: &mut TcpStream) -> Result<()> {
    let input = read_frame(stream)?;
    let mut payload = vec![0_u8; MAX_FRAME];
    let payload_len = state.read_message(&input, &mut payload)?;
    ensure!(payload_len == 0, "unexpected handshake payload");
    Ok(())
}

fn identity_evidence(state: &HandshakeState) -> Result<(String, String)> {
    let remote = state
        .get_remote_static()
        .ok_or_else(|| anyhow!("handshake did not reveal remote static key"))?;
    Ok((sas(state.get_handshake_hash()), hex::encode(remote)))
}

fn check_pin(actual: &str, expected: Option<&str>) -> Result<()> {
    if let Some(expected) = expected {
        ensure!(
            actual.eq_ignore_ascii_case(expected),
            "remote identity pin mismatch: expected {expected}, got {actual}"
        );
    }
    Ok(())
}

fn configure_stream(stream: &TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true)?;
    Ok(())
}

fn run_server(bind: SocketAddr, key_file: PathBuf, expect_remote: Option<String>) -> Result<()> {
    let keys = load_or_generate_keypair(&key_file)?;
    let listener = TcpListener::bind(bind).with_context(|| format!("bind {bind}"))?;
    println!("LISTENING {}", listener.local_addr()?);
    println!("LOCAL_STATIC {}", hex::encode(&keys.public));
    let (mut stream, peer) = listener.accept().context("accept client")?;
    configure_stream(&stream)?;

    let mut state = build_responder(&keys)?;
    read_handshake(&mut state, &mut stream)?;
    write_handshake(&mut state, &mut stream)?;
    read_handshake(&mut state, &mut stream)?;
    ensure!(
        state.is_handshake_finished(),
        "responder handshake incomplete"
    );
    let (sas, remote_static) = identity_evidence(&state)?;
    check_pin(&remote_static, expect_remote.as_deref())?;
    println!("PEER {peer}");
    println!("REMOTE_STATIC {remote_static}");
    println!("SAS {sas}");

    let mut transport = state.into_transport_mode()?;
    let encrypted = read_frame(&mut stream)?;
    let mut plaintext = vec![0_u8; MAX_FRAME];
    let length = transport.read_message(&encrypted, &mut plaintext)?;
    ensure!(
        &plaintext[..length] == b"quick-share-ping",
        "unexpected ping"
    );

    let mut response = vec![0_u8; MAX_FRAME];
    let length = transport.write_message(b"quick-share-pong", &mut response)?;
    write_frame(&mut stream, &response[..length])?;
    println!("ENCRYPTED_PING_OK");
    Ok(())
}

fn run_client(connect: SocketAddr, key_file: PathBuf, expect_remote: Option<String>) -> Result<()> {
    let keys = load_or_generate_keypair(&key_file)?;
    let mut stream = TcpStream::connect_timeout(&connect, Duration::from_secs(5))
        .with_context(|| format!("connect {connect}"))?;
    configure_stream(&stream)?;
    println!("CONNECTED {connect}");
    println!("LOCAL_STATIC {}", hex::encode(&keys.public));

    let mut state = build_initiator(&keys)?;
    write_handshake(&mut state, &mut stream)?;
    read_handshake(&mut state, &mut stream)?;
    write_handshake(&mut state, &mut stream)?;
    ensure!(
        state.is_handshake_finished(),
        "initiator handshake incomplete"
    );
    let (sas, remote_static) = identity_evidence(&state)?;
    check_pin(&remote_static, expect_remote.as_deref())?;
    println!("REMOTE_STATIC {remote_static}");
    println!("SAS {sas}");

    let mut transport = state.into_transport_mode()?;
    let mut encrypted = vec![0_u8; MAX_FRAME];
    let length = transport.write_message(b"quick-share-ping", &mut encrypted)?;
    write_frame(&mut stream, &encrypted[..length])?;

    let response = read_frame(&mut stream)?;
    let mut plaintext = vec![0_u8; MAX_FRAME];
    let length = transport.read_message(&response, &mut plaintext)?;
    ensure!(
        &plaintext[..length] == b"quick-share-pong",
        "unexpected pong"
    );
    println!("ENCRYPTED_PING_OK");
    Ok(())
}

fn run_local(check_mitm: bool) -> Result<()> {
    let initiator = generate_static_keypair()?;
    let responder = generate_static_keypair()?;
    let evidence = complete_xx(&initiator, &responder)?;
    println!("{}", serde_json::to_string_pretty(&evidence)?);

    if check_mitm {
        let (left, right) = mitm_sas_evidence()?;
        println!("MITM endpoint SAS: initiator={left}, responder={right}");
        ensure!(
            left != right,
            "six-digit SAS collision; rerun the experiment"
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Local { check_mitm } => run_local(check_mitm),
        Command::Server {
            bind,
            key_file,
            expect_remote,
        } => run_server(bind, key_file, expect_remote),
        Command::Client {
            connect,
            key_file,
            expect_remote,
        } => run_client(connect, key_file, expect_remote),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_FRAME, read_frame, write_frame};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    #[test]
    fn bounded_frame_roundtrips_over_tcp() {
        // Arrange
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let writer = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            write_frame(&mut stream, b"hello").expect("write frame");
        });
        let (mut stream, _) = listener.accept().expect("accept");

        // Act
        let frame = read_frame(&mut stream).expect("read frame");
        writer.join().expect("writer thread");

        // Assert
        assert_eq!(frame, b"hello");
    }

    #[test]
    fn frame_writer_rejects_oversized_payload() {
        // Arrange
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let connector = thread::spawn(move || TcpStream::connect(address).expect("connect"));
        let (_accepted, _) = listener.accept().expect("accept");
        let mut stream = connector.join().expect("connector");

        // Act
        let error = write_frame(&mut stream, &vec![0_u8; MAX_FRAME + 1]);

        // Assert
        assert!(error.is_err());
    }
}
