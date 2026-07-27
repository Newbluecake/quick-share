//! Fail-closed asynchronous Noise record I/O with wall-clock deadlines.

use crate::noise::{ApplicationFrame, NoiseError, SecureChannel};
use std::{io, time::Duration};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::sync::CancellationToken;

const HANDSHAKE_PACKET_MAX: usize = 4 * 1024;
const CIPHERTEXT_RECORD_MAX: usize = 60 * 1024 + 16;

/// Reads one bounded handshake packet with a wall-clock deadline and closes on any failure.
pub async fn read_handshake_packet<S>(
    stream: &mut S,
    operation_timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, NetworkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if operation_timeout.is_zero() {
        return Err(NetworkError::InvalidTimeout);
    }
    let result = tokio::select! {
        _ = cancellation.cancelled() => Err(NetworkError::Cancelled),
        result = timeout(operation_timeout, read_bounded_record(stream, HANDSHAKE_PACKET_MAX)) => {
            match result {
                Ok(result) => result,
                Err(_) => Err(NetworkError::Timeout),
            }
        }
    };
    match result {
        Ok(packet) => Ok(packet),
        Err(error) => {
            let _ = timeout(operation_timeout, stream.shutdown()).await;
            Err(error)
        }
    }
}

/// Writes one bounded handshake packet with a wall-clock deadline and closes on any failure.
pub async fn write_handshake_packet<S>(
    stream: &mut S,
    packet: &[u8],
    operation_timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<(), NetworkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if operation_timeout.is_zero() {
        return Err(NetworkError::InvalidTimeout);
    }
    let length = match u32::try_from(packet.len()) {
        Ok(length) if !packet.is_empty() && packet.len() <= HANDSHAKE_PACKET_MAX => length,
        _ => {
            let _ = timeout(operation_timeout, stream.shutdown()).await;
            return Err(NetworkError::InvalidRecord);
        }
    };
    let result = tokio::select! {
        _ = cancellation.cancelled() => Err(NetworkError::Cancelled),
        result = timeout(operation_timeout, async {
            stream.write_all(&length.to_be_bytes()).await?;
            stream.write_all(packet).await?;
            stream.flush().await?;
            Ok::<_, io::Error>(())
        }) => match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(NetworkError::Io(error)),
            Err(_) => Err(NetworkError::Timeout),
        }
    };
    if let Err(error) = result {
        let _ = timeout(operation_timeout, stream.shutdown()).await;
        return Err(error);
    }
    Ok(())
}

/// A connected stream and its stateful Noise transport.
///
/// Any I/O, timeout, cancellation, or Noise/framing error permanently closes the session.
pub struct NetworkSession<S> {
    stream: S,
    channel: SecureChannel,
    operation_timeout: Duration,
    closed: bool,
}

impl<S> NetworkSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        stream: S,
        channel: SecureChannel,
        operation_timeout: Duration,
    ) -> Result<Self, NetworkError> {
        if operation_timeout.is_zero() {
            return Err(NetworkError::InvalidTimeout);
        }
        Ok(Self {
            stream,
            channel,
            operation_timeout,
            closed: false,
        })
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn set_operation_timeout(
        &mut self,
        operation_timeout: Duration,
    ) -> Result<(), NetworkError> {
        if operation_timeout.is_zero() {
            return Err(NetworkError::InvalidTimeout);
        }
        self.operation_timeout = operation_timeout;
        Ok(())
    }

    /// Writes every encrypted record for one logical frame under one wall-clock deadline.
    pub async fn send_frame(
        &mut self,
        frame: &ApplicationFrame,
        cancellation: &CancellationToken,
    ) -> Result<(), NetworkError> {
        self.ensure_open()?;
        let records = match self.channel.seal_frame(frame) {
            Ok(records) => records,
            Err(error) => return self.close_with(NetworkError::Noise(error)).await,
        };
        let deadline = Instant::now() + self.operation_timeout;
        for record in records {
            let length = u32::try_from(record.len()).map_err(|_| NetworkError::InvalidRecord)?;
            let result = tokio::select! {
                _ = cancellation.cancelled() => Err(NetworkError::Cancelled),
                result = timeout_at(deadline, async {
                    self.stream.write_all(&length.to_be_bytes()).await?;
                    self.stream.write_all(&record).await?;
                    self.stream.flush().await?;
                    Ok::<_, io::Error>(())
                }) => match result {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(NetworkError::Io(error)),
                    Err(_) => Err(NetworkError::Timeout),
                },
            };
            if let Err(error) = result {
                return self.close_with(error).await;
            }
        }
        Ok(())
    }

    /// Reads and reassembles one logical frame under one wall-clock deadline.
    pub async fn receive_frame(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<ApplicationFrame, NetworkError> {
        self.ensure_open()?;
        let deadline = Instant::now() + self.operation_timeout;
        loop {
            let record = match tokio::select! {
                _ = cancellation.cancelled() => Err(NetworkError::Cancelled),
                result = timeout_at(deadline, read_record(&mut self.stream)) => match result {
                    Ok(Ok(record)) => Ok(record),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(NetworkError::Timeout),
                },
            } {
                Ok(record) => record,
                Err(error) => return self.close_with(error).await,
            };
            match self.channel.open_record(&record) {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {}
                Err(error) => return self.close_with(NetworkError::Noise(error)).await,
            }
        }
    }

    pub async fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = timeout(self.operation_timeout, self.stream.shutdown()).await;
        }
    }

    fn ensure_open(&self) -> Result<(), NetworkError> {
        if self.closed {
            Err(NetworkError::Closed)
        } else {
            Ok(())
        }
    }

    async fn close_with<T>(&mut self, error: NetworkError) -> Result<T, NetworkError> {
        self.close().await;
        Err(error)
    }
}

async fn read_record<S>(stream: &mut S) -> Result<Vec<u8>, NetworkError>
where
    S: AsyncRead + Unpin,
{
    read_bounded_record(stream, CIPHERTEXT_RECORD_MAX).await
}

async fn read_bounded_record<S>(stream: &mut S, maximum: usize) -> Result<Vec<u8>, NetworkError>
where
    S: AsyncRead + Unpin,
{
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await?;
    let length =
        usize::try_from(u32::from_be_bytes(length)).map_err(|_| NetworkError::InvalidRecord)?;
    if length == 0 || length > maximum {
        return Err(NetworkError::InvalidRecord);
    }
    let mut record = vec![0_u8; length];
    stream.read_exact(&mut record).await?;
    Ok(record)
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("network session is already closed")]
    Closed,
    #[error("network operation exceeded its wall-clock deadline")]
    Timeout,
    #[error("network operation was cancelled")]
    Cancelled,
    #[error("network record is empty or exceeds its hard bound")]
    InvalidRecord,
    #[error("network operation timeout must be non-zero")]
    InvalidTimeout,
    #[error("network I/O failed")]
    Io(#[from] io::Error),
    #[error("Noise transport failed and the session was closed")]
    Noise(#[source] NoiseError),
}
