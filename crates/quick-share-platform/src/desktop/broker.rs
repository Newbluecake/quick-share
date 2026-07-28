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

/// Source request held by the Windows event loop while the custom chooser is visible.
#[cfg(any(windows, test))]
pub(crate) struct PendingSourceRequest {
    envelope: Envelope,
}

#[cfg(any(windows, test))]
impl fmt::Debug for PendingSourceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingSourceRequest")
            .field("request", &"REDACTED")
            .field("expires_at", &self.envelope.expires_at)
            .finish()
    }
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

#[cfg(any(windows, test))]
impl PendingSourceRequest {
    pub(crate) fn request(&self) -> &SourceDialog {
        let Request::Source(request) = &self.envelope.request else {
            unreachable!("pending source request contains a different request kind");
        };
        request
    }

    #[cfg(windows)]
    pub(crate) const fn expires_at(&self) -> Instant {
        self.envelope.expires_at
    }

    pub(crate) fn respond(self, response: Result<SourceChoice, DesktopError>) {
        if Instant::now() >= self.envelope.expires_at {
            self.envelope.respond(Err(DesktopError::TimedOut));
        } else {
            self.envelope.respond(response.map(Response::Source));
        }
    }
}

impl DesktopBrokerReceiver {
    #[cfg(any(windows, test))]
    /// Executes queued non-source requests and yields a source request for custom event-loop UI.
    pub(crate) fn drain_for_source_picker(
        &self,
        backend: &dyn DesktopInteraction,
    ) -> Option<PendingSourceRequest> {
        if self.state.load(Ordering::Acquire) == EXITED {
            self.shutdown();
            return None;
        }
        loop {
            match self.receiver.try_recv() {
                Ok(envelope) if matches!(&envelope.request, Request::Source(_)) => {
                    if Instant::now() >= envelope.expires_at {
                        envelope.respond(Err(DesktopError::TimedOut));
                        continue;
                    }
                    return Some(PendingSourceRequest { envelope });
                }
                Ok(envelope) => envelope.execute(backend),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, thread};

    #[derive(Debug, Clone, Copy)]
    struct CancellingDesktop;

    impl DesktopInteraction for CancellingDesktop {
        fn authorize_peer(
            &self,
            _request: &AuthorizationDialog,
        ) -> Result<AuthorizationChoice, DesktopError> {
            Ok(AuthorizationChoice::Cancelled)
        }

        fn choose_send_source(
            &self,
            _request: &SourceDialog,
        ) -> Result<SourceChoice, DesktopError> {
            Ok(SourceChoice::Cancelled)
        }

        fn confirm_receive_directory(
            &self,
            _request: &ReceiveDirectoryDialog,
        ) -> Result<DirectoryChoice, DesktopError> {
            Ok(DirectoryChoice::Cancelled)
        }

        fn resolve_conflict(
            &self,
            _request: &ConflictDialog,
        ) -> Result<ConflictChoice, DesktopError> {
            Ok(ConflictChoice::Cancelled)
        }

        fn notify(&self, _notification: &DesktopNotification) -> Result<(), DesktopError> {
            Ok(())
        }
    }

    fn source_request() -> SourceDialog {
        SourceDialog {
            requester_name: "peer".to_owned(),
            initial_directory: PathBuf::from("C:/Users/test/Downloads"),
        }
    }

    #[test]
    fn pending_source_can_be_completed_by_event_loop_ui() {
        let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
        let wake = Arc::new(move || {
            wake_sender
                .try_send(())
                .map_err(|_| DesktopError::EventLoopExited)
        });
        let (broker, receiver) = desktop_broker(1, Duration::from_secs(1), wake);
        let request = source_request();
        let worker = thread::spawn(move || broker.choose_send_source(&request));
        wake_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("source request wakes event loop");

        let pending = receiver
            .drain_for_source_picker(&CancellingDesktop)
            .expect("source request is yielded");
        assert_eq!(pending.request().requester_name, "peer");
        pending.respond(Ok(SourceChoice::Cancelled));

        assert_eq!(
            worker.join().expect("worker joins"),
            Ok(SourceChoice::Cancelled)
        );
    }

    #[test]
    fn pending_source_rejects_a_selection_that_arrives_after_deadline() {
        let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
        let wake = Arc::new(move || {
            wake_sender
                .try_send(())
                .map_err(|_| DesktopError::EventLoopExited)
        });
        let (broker, receiver) = desktop_broker(1, Duration::from_millis(25), wake);
        let request = source_request();
        let worker = thread::spawn(move || broker.choose_send_source(&request));
        wake_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("source request wakes event loop");
        let pending = receiver
            .drain_for_source_picker(&CancellingDesktop)
            .expect("source request is yielded");
        thread::sleep(Duration::from_millis(35));
        pending.respond(Ok(SourceChoice::Folder(PathBuf::from("C:/late"))));

        assert_eq!(
            worker.join().expect("worker joins"),
            Err(DesktopError::TimedOut)
        );
    }
}
