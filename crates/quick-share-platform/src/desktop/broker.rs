use super::{
    AuthorizationChoice, AuthorizationDialog, ConflictChoice, ConflictDialog, DesktopError,
    DesktopInteraction, DesktopNotification, DirectoryChoice, ReceiveDirectoryDialog, SourceChoice,
    SourceDialog,
};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    time::{Duration, Instant},
};

const RUNNING: u8 = 0;
const EXITED: u8 = 1;
const MAX_CAPACITY: usize = 64;
const MAX_DEADLINE: Duration = Duration::from_secs(60 * 60);

/// Non-blocking wakeup boundary used to notify the owner event loop.
pub trait DesktopWake: Send + Sync {
    fn wake(&self) -> Result<(), DesktopError>;
}

impl<F> DesktopWake for F
where
    F: Fn() -> Result<(), DesktopError> + Send + Sync,
{
    fn wake(&self) -> Result<(), DesktopError> {
        self()
    }
}

/// Thread-safe desktop handle. Native dialogs are executed only by its paired receiver.
#[derive(Clone)]
pub struct DesktopBroker {
    sender: SyncSender<Envelope>,
    wake: Arc<dyn DesktopWake>,
    active: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    deadline: Duration,
}

impl fmt::Debug for DesktopBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopBroker")
            .field("active", &self.active.load(Ordering::Acquire))
            .field("exited", &(self.state.load(Ordering::Acquire) == EXITED))
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

/// Event-loop side of a desktop broker.
pub struct DesktopBrokerReceiver {
    receiver: Receiver<Envelope>,
    state: Arc<AtomicU8>,
}

impl fmt::Debug for DesktopBrokerReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopBrokerReceiver")
            .field("exited", &(self.state.load(Ordering::Acquire) == EXITED))
            .finish_non_exhaustive()
    }
}

/// Creates a bounded, single-flight desktop broker and its event-loop receiver.
#[must_use]
pub fn desktop_broker(
    capacity: usize,
    deadline: Duration,
    wake: Arc<dyn DesktopWake>,
) -> (DesktopBroker, DesktopBrokerReceiver) {
    assert!(
        (1..=MAX_CAPACITY).contains(&capacity),
        "desktop broker capacity must be between one and {MAX_CAPACITY}"
    );
    assert!(
        !deadline.is_zero() && deadline <= MAX_DEADLINE,
        "desktop broker deadline must be nonzero and at most one hour"
    );
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let active = Arc::new(AtomicBool::new(false));
    let state = Arc::new(AtomicU8::new(RUNNING));
    (
        DesktopBroker {
            sender,
            wake,
            active: Arc::clone(&active),
            state: Arc::clone(&state),
            deadline,
        },
        DesktopBrokerReceiver { receiver, state },
    )
}

impl DesktopBroker {
    fn submit(&self, request: Request) -> Result<Response, DesktopError> {
        if self.state.load(Ordering::Acquire) == EXITED {
            return Err(DesktopError::EventLoopExited);
        }
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DesktopError::Busy)?;
        let lease = ActiveLease(Arc::clone(&self.active));
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let envelope = Envelope {
            request,
            response: response_sender,
            expires_at: Instant::now() + self.deadline,
            _lease: lease,
        };
        match self.sender.try_send(envelope) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(DesktopError::Busy),
            Err(TrySendError::Disconnected(_)) => {
                self.state.store(EXITED, Ordering::Release);
                return Err(DesktopError::EventLoopExited);
            }
        }
        if let Err(error) = self.wake.wake() {
            self.state.store(EXITED, Ordering::Release);
            return Err(error);
        }
        match response_receiver.recv_timeout(self.deadline) {
            Ok(response) => response,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(DesktopError::TimedOut),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(DesktopError::EventLoopExited),
        }
    }
}

impl DesktopInteraction for DesktopBroker {
    fn authorize_peer(
        &self,
        request: &AuthorizationDialog,
    ) -> Result<AuthorizationChoice, DesktopError> {
        match self.submit(Request::Authorize(request.clone()))? {
            Response::Authorization(choice) => Ok(choice),
            _ => Err(DesktopError::Backend),
        }
    }

    fn choose_send_source(&self, request: &SourceDialog) -> Result<SourceChoice, DesktopError> {
        match self.submit(Request::Source(request.clone()))? {
            Response::Source(choice) => Ok(choice),
            _ => Err(DesktopError::Backend),
        }
    }

    fn confirm_receive_directory(
        &self,
        request: &ReceiveDirectoryDialog,
    ) -> Result<DirectoryChoice, DesktopError> {
        match self.submit(Request::Directory(request.clone()))? {
            Response::Directory(choice) => Ok(choice),
            _ => Err(DesktopError::Backend),
        }
    }

    fn resolve_conflict(&self, request: &ConflictDialog) -> Result<ConflictChoice, DesktopError> {
        match self.submit(Request::Conflict(request.clone()))? {
            Response::Conflict(choice) => Ok(choice),
            _ => Err(DesktopError::Backend),
        }
    }

    fn notify(&self, notification: &DesktopNotification) -> Result<(), DesktopError> {
        match self.submit(Request::Notify(notification.clone()))? {
            Response::Notified => Ok(()),
            _ => Err(DesktopError::Backend),
        }
    }
}

impl DesktopBrokerReceiver {
    /// Executes all currently queued requests on the caller's event-loop thread.
    pub fn drain(&self, backend: &dyn DesktopInteraction) -> usize {
        if self.state.load(Ordering::Acquire) == EXITED {
            self.shutdown();
            return 0;
        }
        let mut drained = 0;
        loop {
            match self.receiver.try_recv() {
                Ok(envelope) => {
                    drained += 1;
                    envelope.execute(backend);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return drained,
            }
        }
    }

    /// Fails all queued and future requests when the owner event loop exits.
    pub fn shutdown(&self) {
        self.state.store(EXITED, Ordering::Release);
        while let Ok(envelope) = self.receiver.try_recv() {
            envelope.respond(Err(DesktopError::EventLoopExited));
        }
    }
}

impl Drop for DesktopBrokerReceiver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug)]
struct ActiveLease(Arc<AtomicBool>);

impl Drop for ActiveLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
struct Envelope {
    request: Request,
    response: SyncSender<Result<Response, DesktopError>>,
    expires_at: Instant,
    _lease: ActiveLease,
}

impl Envelope {
    fn execute(self, backend: &dyn DesktopInteraction) {
        if Instant::now() >= self.expires_at {
            self.respond(Err(DesktopError::TimedOut));
            return;
        }
        let response = match &self.request {
            Request::Authorize(request) => {
                backend.authorize_peer(request).map(Response::Authorization)
            }
            Request::Source(request) => backend.choose_send_source(request).map(Response::Source),
            Request::Directory(request) => backend
                .confirm_receive_directory(request)
                .map(Response::Directory),
            Request::Conflict(request) => backend.resolve_conflict(request).map(Response::Conflict),
            Request::Notify(request) => backend.notify(request).map(|()| Response::Notified),
        };
        self.respond(response);
    }

    fn respond(self, response: Result<Response, DesktopError>) {
        let _ = self.response.try_send(response);
    }
}

#[derive(Debug)]
enum Request {
    Authorize(AuthorizationDialog),
    Source(SourceDialog),
    Directory(ReceiveDirectoryDialog),
    Conflict(ConflictDialog),
    Notify(DesktopNotification),
}

#[derive(Debug)]
enum Response {
    Authorization(AuthorizationChoice),
    Source(SourceChoice),
    Directory(DirectoryChoice),
    Conflict(ConflictChoice),
    Notified,
}
