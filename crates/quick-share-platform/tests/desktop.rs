use quick_share_platform::desktop::{
    AuthorizationChoice, AuthorizationDialog, ConflictChoice, ConflictDialog, DesktopError,
    DesktopInteraction, DesktopNotification, DirectoryChoice, ReceiveDirectoryDialog, SourceChoice,
    SourceDialog, UnavailableDesktop, desktop_broker,
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

#[test]
fn unavailable_desktop_fails_every_operation_without_fabricating_a_choice() {
    let desktop = UnavailableDesktop;

    assert!(matches!(
        desktop.authorize_peer(&AuthorizationDialog {
            device_name: "peer".to_owned(),
            device_id: "qs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            sas: "123456".to_owned(),
            identity_changed: false,
        }),
        Err(DesktopError::Unsupported)
    ));
    assert!(matches!(
        desktop.choose_send_source(&SourceDialog {
            requester_name: "peer".to_owned(),
            initial_directory: std::env::current_dir().expect("current directory"),
        }),
        Err(DesktopError::Unsupported)
    ));
    assert!(matches!(
        desktop.confirm_receive_directory(&ReceiveDirectoryDialog {
            sender_name: "peer".to_owned(),
            current_directory: PathBuf::from("C:/Downloads"),
        }),
        Err(DesktopError::Unsupported)
    ));
    assert!(matches!(
        desktop.resolve_conflict(&ConflictDialog {
            relative_path: "file.txt".to_owned(),
            directory: false,
        }),
        Err(DesktopError::Unsupported)
    ));
    assert!(matches!(
        desktop.notify(&DesktopNotification {
            title: "Quick Share".to_owned(),
            message: "request waiting".to_owned(),
        }),
        Err(DesktopError::Unsupported)
    ));
}

#[derive(Debug, Clone, Copy)]
struct AcceptingDesktop;

impl DesktopInteraction for AcceptingDesktop {
    fn authorize_peer(
        &self,
        _request: &AuthorizationDialog,
    ) -> Result<AuthorizationChoice, DesktopError> {
        Ok(AuthorizationChoice::AcceptOnce)
    }

    fn choose_send_source(&self, _request: &SourceDialog) -> Result<SourceChoice, DesktopError> {
        Ok(SourceChoice::Cancelled)
    }

    fn confirm_receive_directory(
        &self,
        _request: &ReceiveDirectoryDialog,
    ) -> Result<DirectoryChoice, DesktopError> {
        Ok(DirectoryChoice::Cancelled)
    }

    fn resolve_conflict(&self, _request: &ConflictDialog) -> Result<ConflictChoice, DesktopError> {
        Ok(ConflictChoice::Cancelled)
    }

    fn notify(&self, _notification: &DesktopNotification) -> Result<(), DesktopError> {
        Ok(())
    }
}

#[test]
fn broker_marshals_to_receiver_and_rejects_overlapping_modal_work() {
    let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
    let wake = Arc::new(move || {
        wake_sender
            .try_send(())
            .map_err(|_| DesktopError::EventLoopExited)
    });
    let (broker, receiver) = desktop_broker(1, Duration::from_secs(2), wake);
    let request = AuthorizationDialog {
        device_name: "peer".to_owned(),
        device_id: "qs_device".to_owned(),
        sas: "123456".to_owned(),
        identity_changed: false,
    };
    let worker_broker = broker.clone();
    let worker_request = request.clone();
    let worker = thread::spawn(move || worker_broker.authorize_peer(&worker_request));

    wake_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("event-loop wake");
    assert_eq!(broker.authorize_peer(&request), Err(DesktopError::Busy));
    assert_eq!(receiver.drain(&AcceptingDesktop), 1);
    assert_eq!(
        worker.join().expect("worker joins"),
        Ok(AuthorizationChoice::AcceptOnce)
    );
}

#[test]
fn broker_deadline_and_event_loop_exit_fail_closed_without_stale_dialogs() {
    let (broker, receiver) = desktop_broker(1, Duration::from_millis(20), Arc::new(|| Ok(())));
    let request = SourceDialog {
        requester_name: "peer".to_owned(),
        initial_directory: std::env::current_dir().expect("current directory"),
    };
    assert_eq!(
        broker.choose_send_source(&request),
        Err(DesktopError::TimedOut)
    );
    assert_eq!(broker.choose_send_source(&request), Err(DesktopError::Busy));
    assert_eq!(receiver.drain(&AcceptingDesktop), 1);

    let queued_broker = broker.clone();
    let worker = thread::spawn(move || queued_broker.choose_send_source(&request));
    thread::sleep(Duration::from_millis(2));
    receiver.shutdown();
    assert_eq!(
        worker.join().expect("worker joins"),
        Err(DesktopError::EventLoopExited)
    );
    assert_eq!(
        broker.notify(&DesktopNotification {
            title: "Quick Share".to_owned(),
            message: "done".to_owned(),
        }),
        Err(DesktopError::EventLoopExited)
    );
}

#[test]
fn broker_wake_failure_marks_event_loop_exited_and_never_opens_stale_work() {
    let (broker, receiver) = desktop_broker(
        1,
        Duration::from_secs(1),
        Arc::new(|| Err(DesktopError::EventLoopExited)),
    );
    let notification = DesktopNotification {
        title: "Quick Share".to_owned(),
        message: "waiting".to_owned(),
    };
    assert_eq!(
        broker.notify(&notification),
        Err(DesktopError::EventLoopExited)
    );
    assert_eq!(receiver.drain(&AcceptingDesktop), 0);
    assert_eq!(
        broker.notify(&notification),
        Err(DesktopError::EventLoopExited)
    );
}

#[test]
fn desktop_choices_preserve_cancel_as_a_normal_result_and_redact_local_paths() {
    let files = SourceChoice::Files(vec![PathBuf::from("C:/secret/file.txt")]);
    let folder = SourceChoice::Folder(PathBuf::from("C:/secret/folder"));
    let directory = DirectoryChoice::Confirm(PathBuf::from("C:/secret/downloads"));

    assert_eq!(
        files
            .selected_paths()
            .expect("files")
            .expect("selection")
            .len(),
        1
    );
    assert_eq!(
        folder
            .selected_paths()
            .expect("folder")
            .expect("selection")
            .len(),
        1
    );
    assert_eq!(
        SourceChoice::Cancelled.selected_paths().expect("cancel"),
        None
    );
    assert!(SourceChoice::Files(Vec::new()).selected_paths().is_err());
    assert!(matches!(
        AuthorizationChoice::Cancelled,
        AuthorizationChoice::Cancelled
    ));
    assert!(matches!(
        ConflictChoice::Cancelled,
        ConflictChoice::Cancelled
    ));
    assert!(matches!(
        DirectoryChoice::Cancelled,
        DirectoryChoice::Cancelled
    ));

    let conflict = ConflictDialog {
        relative_path: "secret/file.txt".to_owned(),
        directory: false,
    };
    for diagnostic in [
        format!("{files:?}"),
        format!("{folder:?}"),
        format!("{directory:?}"),
        format!("{conflict:?}"),
    ] {
        assert!(!diagnostic.contains("secret"));
        assert!(diagnostic.contains("REDACTED"));
    }
}
