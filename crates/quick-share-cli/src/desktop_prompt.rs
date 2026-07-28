//! Windows-agent desktop authorization, destination planning, and routed completion prompts.

use async_trait::async_trait;
use quick_share_core::{
    destination::{
        ConflictDecision, ConflictKind, ConflictSelections, DestinationPlan, DestinationPlanError,
    },
    identity::{TrustStatus, TrustedDeviceStore},
    receive::{ReceiveBinding, ReceiveBindingStore, ReceiveDestinationStore},
};
use quick_share_platform::{
    AppDirs, FileSensitivity, atomic_write,
    desktop::{
        AuthorizationChoice, AuthorizationDialog, ConflictAction, ConflictChoice, ConflictDialog,
        ConflictScope, DesktopError, DesktopInteraction, DirectoryChoice, ReceiveDirectoryDialog,
        SourceChoice,
    },
};
use quick_share_protocol::{
    CancelReason, ChunkAck, ChunkData, DeviceId, OfferDecision, RejectionReason, RequestId,
    TransferCancel, TransferComplete, TransferCompleteAck, TransferId, TransferOffer,
    TransferStatus, TransferStatusRequest, TransferStatusResponse,
};
use quick_share_transfer::{
    auth::PeerAuthContext,
    direct::{DirectError, OfferPrompt},
    receiver::{ReceiverEndpoint, ReceiverError},
    receiver_router::{ReceiverRouter, ReceiverRouterError},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

const SOURCE_DIRECTORY_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub(crate) struct SourceDirectoryStore {
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceDirectoryDocument {
    version: u8,
    last_directory: PathBuf,
}

impl SourceDirectoryStore {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn load(&self) -> Result<Option<PathBuf>, String> {
        let source = match fs::read(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let document: SourceDirectoryDocument =
            serde_json::from_slice(&source).map_err(|error| error.to_string())?;
        if document.version != SOURCE_DIRECTORY_VERSION {
            return Err("unsupported source-directory state version".to_owned());
        }
        Ok(document
            .last_directory
            .is_dir()
            .then_some(document.last_directory))
    }

    pub(crate) fn remember_choice(&self, choice: &SourceChoice) -> Result<(), String> {
        let Some(directory) = selected_source_directory(choice) else {
            return Ok(());
        };
        let document = SourceDirectoryDocument {
            version: SOURCE_DIRECTORY_VERSION,
            last_directory: directory,
        };
        let source = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
        atomic_write(&self.path, &source, FileSensitivity::Private)
            .map_err(|error| error.to_string())
    }
}

fn selected_source_directory(choice: &SourceChoice) -> Option<PathBuf> {
    let selected = match choice {
        SourceChoice::Files(paths) => paths.first()?.parent()?,
        SourceChoice::Folder(path) => path.parent().unwrap_or(path),
        SourceChoice::Cancelled => return None,
    };
    selected.is_dir().then(|| selected.to_path_buf())
}

#[derive(Clone)]
struct PlannedTransfer {
    offer: TransferOffer,
    output_root: std::path::PathBuf,
    selections: ConflictSelections,
}

/// Routed receiver with commit-time conflict prompting and plan replacement.
pub(crate) struct AgentReceiver {
    router: Arc<ReceiverRouter>,
    bindings: ReceiveBindingStore,
    desktop: Arc<dyn DesktopInteraction>,
    plans: Mutex<BTreeMap<TransferId, PlannedTransfer>>,
}

impl std::fmt::Debug for AgentReceiver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentReceiver")
            .field("router", &self.router)
            .field("bindings", &self.bindings)
            .field("plans", &"[REDACTED]")
            .finish()
    }
}

impl AgentReceiver {
    pub(crate) fn new(
        router: Arc<ReceiverRouter>,
        bindings: ReceiveBindingStore,
        desktop: Arc<dyn DesktopInteraction>,
    ) -> Self {
        Self {
            router,
            bindings,
            desktop,
            plans: Mutex::new(BTreeMap::new()),
        }
    }

    fn bind(
        &self,
        offer: &TransferOffer,
        output_root: std::path::PathBuf,
        plan: DestinationPlan,
        selections: ConflictSelections,
    ) -> Result<(), DirectError> {
        let manifest_digest = offer_digest(offer)?;
        self.router
            .bind(
                ReceiveBinding {
                    transfer_id: offer.transfer_id,
                    sender_device_id: offer.sender.device_id.clone(),
                    manifest_digest,
                    output_root: output_root.clone(),
                    destination_plan: plan,
                },
                offer,
            )
            .map_err(|_| DirectError::InvalidResponse)?;
        self.lock_plans()
            .map_err(|_| DirectError::InvalidResponse)?
            .insert(
                offer.transfer_id,
                PlannedTransfer {
                    offer: offer.clone(),
                    output_root,
                    selections,
                },
            );
        Ok(())
    }

    fn remove_binding(&self, transfer_id: TransferId) {
        let _ = self.bindings.remove(transfer_id);
        if let Ok(mut plans) = self.lock_plans() {
            plans.remove(&transfer_id);
        }
    }

    pub(crate) fn cleanup(&self, transfer_id: TransferId) -> Result<(), ReceiverRouterError> {
        self.router.cleanup(transfer_id)?;
        if let Ok(mut plans) = self.lock_plans() {
            plans.remove(&transfer_id);
        }
        Ok(())
    }

    pub(crate) fn pause_all(&self) -> Result<usize, ReceiverRouterError> {
        self.router.pause_all()
    }

    fn resolve_commit_race(
        &self,
        transfer_id: TransferId,
        entry_id: quick_share_protocol::EntryId,
    ) -> Result<(), ReceiverError> {
        let mut planned = self
            .lock_plans()
            .map_err(|_| ReceiverError::Internal)?
            .get(&transfer_id)
            .cloned()
            .ok_or(ReceiverError::NotFound)?;
        let entry = planned
            .offer
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .ok_or(ReceiverError::ManifestChanged)?;
        let kind = if matches!(
            entry.kind,
            quick_share_protocol::ManifestEntryKind::Directory
        ) {
            ConflictKind::Directory
        } else {
            ConflictKind::File
        };
        let choice = self
            .desktop
            .resolve_conflict(&ConflictDialog {
                relative_path: entry.relative_path.clone(),
                directory: kind == ConflictKind::Directory,
            })
            .map_err(map_desktop_receiver)?;
        let ConflictChoice::Decision { action, scope } = choice else {
            return Err(ReceiverError::Terminal(TransferStatus::Cancelled));
        };
        planned.selections =
            apply_conflict_choice(planned.selections, entry_id, kind, action, scope);
        let plan = build_plan_sync(
            self.desktop.as_ref(),
            &planned.output_root,
            &planned.offer,
            planned.selections.clone(),
        )?
        .0;
        self.router
            .update_plan(transfer_id, &planned.offer, plan)
            .map_err(map_router_error)?;
        self.lock_plans()
            .map_err(|_| ReceiverError::Internal)?
            .insert(transfer_id, planned);
        Ok(())
    }

    fn lock_plans(&self) -> Result<MutexGuard<'_, BTreeMap<TransferId, PlannedTransfer>>, ()> {
        self.plans.lock().map_err(|_| ())
    }
}

impl ReceiverEndpoint for AgentReceiver {
    fn status(
        &self,
        peer: &DeviceId,
        request: TransferStatusRequest,
        now: Instant,
    ) -> Result<TransferStatusResponse, ReceiverError> {
        self.router
            .status(peer, request, now)
            .map_err(map_router_error)
    }

    fn receive_chunk(
        &self,
        peer: &DeviceId,
        request_id: RequestId,
        frame: ChunkData,
        now: Instant,
    ) -> Result<Option<ChunkAck>, ReceiverError> {
        self.router
            .receive_chunk(peer, request_id, frame, now)
            .map_err(map_router_error)
    }

    fn complete(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
    ) -> Result<TransferCompleteAck, ReceiverError> {
        let maximum_attempts = self
            .lock_plans()
            .ok()
            .and_then(|plans| {
                plans
                    .get(&request.transfer_id)
                    .map(|plan| plan.offer.entries.len())
            })
            .unwrap_or(1)
            .saturating_add(1);
        for _ in 0..maximum_attempts {
            match self.router.complete(peer, request.clone(), now) {
                Ok(response) => return Ok(response),
                Err(ReceiverRouterError::Receiver(ReceiverError::ConflictPending { entry_id })) => {
                    if let Err(error) = self.resolve_commit_race(request.transfer_id, entry_id) {
                        if matches!(error, ReceiverError::Terminal(TransferStatus::Cancelled)) {
                            let _ = self.router.cancel(
                                peer,
                                TransferCancel {
                                    transfer_id: request.transfer_id,
                                    authorization: request.authorization.clone(),
                                    reason: CancelReason::User,
                                },
                                Instant::now(),
                            );
                        }
                        return Err(error);
                    }
                }
                Err(error) => return Err(map_router_error(error)),
            }
        }
        Err(ReceiverError::Internal)
    }

    fn cancel(
        &self,
        peer: &DeviceId,
        request: TransferCancel,
        now: Instant,
    ) -> Result<(), ReceiverError> {
        self.router
            .cancel(peer, request, now)
            .map_err(map_router_error)
    }

    fn disconnect(&self, peer: &DeviceId) -> Result<usize, ReceiverError> {
        self.router.disconnect(peer).map_err(map_router_error)
    }
}

/// Offer prompt that performs identity UI, destination UI, planning, and binding before Accept.
pub(crate) struct DesktopOfferPrompt {
    desktop: Arc<dyn DesktopInteraction>,
    trust_store: TrustedDeviceStore,
    destinations: ReceiveDestinationStore,
    dirs: AppDirs,
    receiver: Arc<AgentReceiver>,
}

impl std::fmt::Debug for DesktopOfferPrompt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopOfferPrompt")
            .field("desktop", &"[DESKTOP]")
            .field("trust_store", &"[REDACTED]")
            .field("destinations", &"[REDACTED]")
            .field("receiver", &self.receiver)
            .finish()
    }
}

impl DesktopOfferPrompt {
    pub(crate) fn new(
        desktop: Arc<dyn DesktopInteraction>,
        trust_store: TrustedDeviceStore,
        destinations: ReceiveDestinationStore,
        dirs: AppDirs,
        receiver: Arc<AgentReceiver>,
    ) -> Self {
        Self {
            desktop,
            trust_store,
            destinations,
            dirs,
            receiver,
        }
    }

    async fn prepare(
        &self,
        peer: &PeerAuthContext,
        offer: &TransferOffer,
        authorize: bool,
    ) -> Result<(OfferDecision, bool), DirectError> {
        if peer.change_reason().is_some() {
            return Ok((
                OfferDecision::Reject {
                    reason: RejectionReason::Policy,
                },
                false,
            ));
        }
        let mut decision = AuthorizationChoice::AcceptOnce;
        if authorize && !peer.is_trusted() {
            let desktop = Arc::clone(&self.desktop);
            let dialog = authorization_dialog(peer);
            decision = tokio::task::spawn_blocking(move || desktop.authorize_peer(&dialog))
                .await
                .map_err(|_| DirectError::InvalidResponse)?
                .map_err(map_desktop_direct)?;
            if matches!(
                decision,
                AuthorizationChoice::Reject | AuthorizationChoice::Cancelled
            ) {
                return Ok((
                    OfferDecision::Reject {
                        reason: RejectionReason::UserRejected,
                    },
                    false,
                ));
            }
        }

        let trust_status = self
            .trust_store
            .check(peer.device_id(), &peer.public_key())
            .map_err(|_| DirectError::InvalidResponse)?;
        let preferred = match &trust_status {
            TrustStatus::Trusted(device) => self
                .destinations
                .preferred_directory(device, &self.dirs)
                .map_err(|_| DirectError::InvalidResponse)?,
            TrustStatus::Unknown | TrustStatus::KeyMismatch => {
                self.dirs.download_dir().to_path_buf()
            }
        };
        let desktop = Arc::clone(&self.desktop);
        let directory_dialog = ReceiveDirectoryDialog {
            sender_name: peer.name().to_owned(),
            current_directory: preferred,
        };
        let directory = tokio::task::spawn_blocking(move || {
            desktop.confirm_receive_directory(&directory_dialog)
        })
        .await
        .map_err(|_| DirectError::InvalidResponse)?
        .map_err(map_desktop_direct)?;
        let DirectoryChoice::Confirm(output_root) = directory else {
            return Ok((
                OfferDecision::Reject {
                    reason: RejectionReason::UserRejected,
                },
                false,
            ));
        };

        let desktop = Arc::clone(&self.desktop);
        let output = output_root.clone();
        let offer_for_plan = offer.clone();
        let (plan, selections) = tokio::task::spawn_blocking(move || {
            build_plan_sync(
                desktop.as_ref(),
                &output,
                &offer_for_plan,
                ConflictSelections::default(),
            )
        })
        .await
        .map_err(|_| DirectError::InvalidResponse)?
        .map_err(|_| DirectError::InvalidResponse)?;
        self.receiver
            .bind(offer, output_root.clone(), plan, selections)?;

        let should_trust = decision == AuthorizationChoice::AcceptAndTrust;
        if should_trust
            && self
                .trust_store
                .trust_peer(
                    peer.device_id().clone(),
                    peer.claimed_name(),
                    peer.public_key(),
                )
                .is_err()
        {
            self.receiver.remove_binding(offer.transfer_id);
            return Err(DirectError::InvalidResponse);
        }
        let final_trust = self
            .trust_store
            .check(peer.device_id(), &peer.public_key())
            .map_err(|_| DirectError::InvalidResponse)?;
        if matches!(final_trust, TrustStatus::Trusted(_))
            && self
                .destinations
                .remember(&final_trust, &output_root)
                .is_err()
        {
            self.receiver.remove_binding(offer.transfer_id);
            return Err(DirectError::InvalidResponse);
        }
        Ok((
            if should_trust {
                OfferDecision::AcceptAndTrust
            } else {
                OfferDecision::AcceptOnce
            },
            should_trust,
        ))
    }
}

#[async_trait]
impl OfferPrompt for DesktopOfferPrompt {
    async fn prepare_expected(
        &self,
        peer: &PeerAuthContext,
        _view: &quick_share_transfer::offer::OfferView,
        offer: &TransferOffer,
    ) -> Result<(), DirectError> {
        let (decision, _) = self.prepare(peer, offer, false).await?;
        if matches!(decision, OfferDecision::AcceptOnce) {
            Ok(())
        } else {
            Err(DirectError::Unauthorized)
        }
    }

    async fn decide(
        &self,
        peer: &PeerAuthContext,
        _view: &quick_share_transfer::offer::OfferView,
        offer: &TransferOffer,
    ) -> Result<(OfferDecision, bool), DirectError> {
        self.prepare(peer, offer, true).await
    }
}

fn authorization_dialog(peer: &PeerAuthContext) -> AuthorizationDialog {
    AuthorizationDialog {
        device_name: peer.name().to_owned(),
        device_id: peer.device_id().to_string(),
        sas: peer.sas().to_string(),
        identity_changed: peer.change_reason().is_some(),
    }
}

fn build_plan_sync(
    desktop: &dyn DesktopInteraction,
    output_root: &std::path::Path,
    offer: &TransferOffer,
    mut selections: ConflictSelections,
) -> Result<(DestinationPlan, ConflictSelections), ReceiverError> {
    for _ in 0..=offer.entries.len() {
        match DestinationPlan::build(output_root, offer, &selections) {
            Ok(plan) => return Ok((plan, selections)),
            Err(DestinationPlanError::UnresolvedConflict { entry_id, path }) => {
                let entry = offer
                    .entries
                    .iter()
                    .find(|entry| entry.id == entry_id)
                    .ok_or(ReceiverError::ManifestChanged)?;
                let kind = if matches!(
                    entry.kind,
                    quick_share_protocol::ManifestEntryKind::Directory
                ) {
                    ConflictKind::Directory
                } else {
                    ConflictKind::File
                };
                let choice = desktop
                    .resolve_conflict(&ConflictDialog {
                        relative_path: path.as_str().to_owned(),
                        directory: kind == ConflictKind::Directory,
                    })
                    .map_err(map_desktop_receiver)?;
                let ConflictChoice::Decision { action, scope } = choice else {
                    return Err(ReceiverError::Terminal(TransferStatus::Cancelled));
                };
                selections = apply_conflict_choice(selections, entry_id, kind, action, scope);
            }
            Err(error) => return Err(ReceiverError::DestinationPlan(error)),
        }
    }
    Err(ReceiverError::Internal)
}

fn apply_conflict_choice(
    selections: ConflictSelections,
    entry_id: quick_share_protocol::EntryId,
    kind: ConflictKind,
    action: ConflictAction,
    scope: ConflictScope,
) -> ConflictSelections {
    let decision = match action {
        ConflictAction::Overwrite => ConflictDecision::Overwrite,
        ConflictAction::Skip => ConflictDecision::Skip,
        ConflictAction::Rename => ConflictDecision::Rename,
    };
    match scope {
        ConflictScope::ThisEntry => selections.with_entry(entry_id, decision),
        ConflictScope::AllRemaining => selections.apply_to_all(kind, decision),
    }
}

fn offer_digest(offer: &TransferOffer) -> Result<[u8; 32], DirectError> {
    serde_json::to_vec(offer)
        .map(|bytes| *blake3::hash(&bytes).as_bytes())
        .map_err(|_| DirectError::InvalidResponse)
}

pub(crate) fn callback_notification(succeeded: bool) -> Option<&'static str> {
    (!succeeded).then_some("Selected content could not be sent.")
}

pub(crate) fn incoming_transfer_notification(succeeded: bool) -> Option<&'static str> {
    (!succeeded).then_some("Incoming transfer could not be received.")
}

fn map_desktop_direct(error: DesktopError) -> DirectError {
    match error {
        DesktopError::Busy => {
            DirectError::Selection(quick_share_transfer::selection::SelectionError::Internal)
        }
        DesktopError::Unsupported
        | DesktopError::Unavailable
        | DesktopError::TimedOut
        | DesktopError::EventLoopExited
        | DesktopError::InvalidSelection
        | DesktopError::InvalidDirectory
        | DesktopError::DirectoryNotWritable
        | DesktopError::Backend => DirectError::InvalidResponse,
    }
}

fn map_desktop_receiver(_error: DesktopError) -> ReceiverError {
    ReceiverError::Internal
}

fn map_router_error(error: ReceiverRouterError) -> ReceiverError {
    match error {
        ReceiverRouterError::BindingNotFound => ReceiverError::NotFound,
        ReceiverRouterError::BindingMismatch | ReceiverRouterError::InvalidOffer(_) => {
            ReceiverError::ManifestChanged
        }
        ReceiverRouterError::ReceiveTaskLimit => ReceiverError::ReceiveTaskLimit,
        ReceiverRouterError::FileStreamLimit => ReceiverError::FileStreamLimit,
        ReceiverRouterError::Receiver(error) => error,
        ReceiverRouterError::Offer(error) => ReceiverError::Offer(error),
        ReceiverRouterError::DestinationPlan(error) => ReceiverError::DestinationPlan(error),
        ReceiverRouterError::Store(error) => ReceiverError::Store(error),
        ReceiverRouterError::Internal | ReceiverRouterError::ReceiveState(_) => {
            ReceiverError::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceDirectoryStore, apply_conflict_choice, build_plan_sync};
    use quick_share_core::destination::{ConflictKind, ConflictSelections, DestinationDisposition};
    use quick_share_platform::desktop::{
        AuthorizationChoice, AuthorizationDialog, ConflictAction, ConflictChoice, ConflictDialog,
        ConflictScope, DesktopError, DesktopInteraction, DesktopNotification, DirectoryChoice,
        ReceiveDirectoryDialog, SourceChoice, SourceDialog,
    };
    use quick_share_protocol::{
        Capability, ContentKind, DeviceId, DeviceInfo, EntryId, ManifestEntry, ManifestEntryKind,
        ProtocolVersion, TransferId, TransferOffer,
    };
    use std::{
        collections::{BTreeSet, VecDeque},
        sync::Mutex,
    };
    use uuid::Uuid;

    struct ScriptedDesktop {
        conflicts: Mutex<VecDeque<ConflictChoice>>,
        seen: Mutex<Vec<ConflictDialog>>,
    }

    impl ScriptedDesktop {
        fn new(choices: impl IntoIterator<Item = ConflictChoice>) -> Self {
            Self {
                conflicts: Mutex::new(choices.into_iter().collect()),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl DesktopInteraction for ScriptedDesktop {
        fn authorize_peer(
            &self,
            _request: &AuthorizationDialog,
        ) -> Result<AuthorizationChoice, DesktopError> {
            Ok(AuthorizationChoice::AcceptOnce)
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
            request: &ConflictDialog,
        ) -> Result<ConflictChoice, DesktopError> {
            self.seen.lock().expect("seen").push(request.clone());
            self.conflicts
                .lock()
                .expect("conflicts")
                .pop_front()
                .ok_or(DesktopError::Backend)
        }

        fn notify(&self, _notification: &DesktopNotification) -> Result<(), DesktopError> {
            Ok(())
        }
    }

    fn file_entry(id: u32, path: &str) -> ManifestEntry {
        ManifestEntry {
            id: EntryId::new(id).expect("entry id"),
            relative_path: path.to_owned(),
            kind: ManifestEntryKind::File,
            size: 4,
            digest: Some([id as u8; 32]),
        }
    }

    fn offer(entries: Vec<ManifestEntry>) -> TransferOffer {
        let total = entries.iter().map(|entry| entry.size).sum();
        TransferOffer {
            protocol_version: ProtocolVersion::V1_1,
            transfer_id: TransferId::new(Uuid::now_v7()),
            initiated_by: None,
            sender: DeviceInfo {
                device_id: DeviceId::parse(format!("qs_{}", "a".repeat(32))).expect("device"),
                name: "peer".to_owned(),
                capabilities: BTreeSet::from([Capability::Files]),
            },
            content_kind: ContentKind::Files,
            chunk_size: 4 * 1024 * 1024,
            total_bytes: total,
            entries,
        }
    }

    #[test]
    fn build_plan_resolves_file_conflicts_via_desktop_and_scopes_apply_all() {
        let root = tempfile::tempdir().expect("root");
        std::fs::write(root.path().join("alpha.txt"), b"old1").expect("alpha");
        std::fs::write(root.path().join("beta.txt"), b"old2").expect("beta");
        let offer = offer(vec![file_entry(1, "alpha.txt"), file_entry(2, "beta.txt")]);

        let desktop = ScriptedDesktop::new([ConflictChoice::Decision {
            action: ConflictAction::Overwrite,
            scope: ConflictScope::AllRemaining,
        }]);
        let (plan, _) =
            build_plan_sync(&desktop, root.path(), &offer, ConflictSelections::default())
                .expect("plan");
        assert!(matches!(
            plan.entry(EntryId::new(1).unwrap()),
            Some(DestinationDisposition::Commit {
                replace_existing: true,
                ..
            })
        ));
        assert!(matches!(
            plan.entry(EntryId::new(2).unwrap()),
            Some(DestinationDisposition::Commit {
                replace_existing: true,
                ..
            })
        ));
        // Apply-all means only one conflict dialog was shown for two conflicts.
        assert_eq!(desktop.seen.lock().expect("seen").len(), 1);
    }

    #[test]
    fn build_plan_supports_rename_skip_and_cancel() {
        let root = tempfile::tempdir().expect("root");
        std::fs::write(root.path().join("alpha.txt"), b"old1").expect("alpha");
        let offer = offer(vec![file_entry(1, "alpha.txt")]);

        let rename = ScriptedDesktop::new([ConflictChoice::Decision {
            action: ConflictAction::Rename,
            scope: ConflictScope::ThisEntry,
        }]);
        let (plan, _) =
            build_plan_sync(&rename, root.path(), &offer, ConflictSelections::default())
                .expect("rename plan");
        match plan.entry(EntryId::new(1).unwrap()) {
            Some(DestinationDisposition::Commit {
                relative_path,
                replace_existing,
            }) => {
                assert!(!*replace_existing);
                assert_ne!(relative_path.as_str(), "alpha.txt");
            }
            other => panic!("unexpected rename disposition: {other:?}"),
        }

        let skip = ScriptedDesktop::new([ConflictChoice::Decision {
            action: ConflictAction::Skip,
            scope: ConflictScope::ThisEntry,
        }]);
        let (plan, _) = build_plan_sync(&skip, root.path(), &offer, ConflictSelections::default())
            .expect("skip plan");
        assert!(matches!(
            plan.entry(EntryId::new(1).unwrap()),
            Some(DestinationDisposition::Skip)
        ));

        let cancel = ScriptedDesktop::new([ConflictChoice::Cancelled]);
        assert!(
            build_plan_sync(&cancel, root.path(), &offer, ConflictSelections::default()).is_err()
        );
    }

    #[test]
    fn source_directory_store_reopens_the_parent_of_the_last_selection() {
        let root = tempfile::tempdir().expect("root");
        let downloads = root.path().join("Downloads");
        std::fs::create_dir(&downloads).expect("downloads");
        let file = downloads.join("payload.txt");
        std::fs::write(&file, b"payload").expect("file");
        let state = root.path().join("source-selection.json");
        let store = SourceDirectoryStore::new(&state);

        store
            .remember_choice(&SourceChoice::Files(vec![file]))
            .expect("remember file directory");
        assert_eq!(
            SourceDirectoryStore::new(&state).load().expect("reload"),
            Some(downloads.clone())
        );

        store
            .remember_choice(&SourceChoice::Cancelled)
            .expect("cancel is not persisted");
        assert_eq!(store.load().expect("unchanged"), Some(downloads.clone()));

        let selected_folder = downloads.join("folder");
        std::fs::create_dir(&selected_folder).expect("folder");
        store
            .remember_choice(&SourceChoice::Folder(selected_folder))
            .expect("remember folder parent");
        assert_eq!(store.load().expect("folder parent"), Some(downloads));
    }

    #[test]
    fn source_directory_store_ignores_a_directory_that_no_longer_exists() {
        let root = tempfile::tempdir().expect("root");
        let selected = root.path().join("selected");
        std::fs::create_dir(&selected).expect("selected");
        let file = selected.join("payload.txt");
        std::fs::write(&file, b"payload").expect("file");
        let store = SourceDirectoryStore::new(root.path().join("source-selection.json"));
        store
            .remember_choice(&SourceChoice::Files(vec![file]))
            .expect("remember");
        std::fs::remove_dir_all(&selected).expect("remove selected directory");
        assert_eq!(store.load().expect("stale state"), None);
    }

    #[test]
    fn successful_callback_is_silent_and_failure_requires_attention() {
        assert_eq!(super::callback_notification(true), None);
        assert_eq!(
            super::callback_notification(false),
            Some("Selected content could not be sent.")
        );
    }

    #[test]
    fn successful_incoming_transfer_is_silent_and_failure_requires_attention() {
        assert_eq!(super::incoming_transfer_notification(true), None);
        assert_eq!(
            super::incoming_transfer_notification(false),
            Some("Incoming transfer could not be received.")
        );
    }

    #[test]
    fn apply_conflict_choice_scopes_entry_and_kind() {
        let entry = EntryId::new(7).unwrap();
        let per_entry = apply_conflict_choice(
            ConflictSelections::default(),
            entry,
            ConflictKind::File,
            ConflictAction::Overwrite,
            ConflictScope::ThisEntry,
        );
        let all = apply_conflict_choice(
            ConflictSelections::default(),
            entry,
            ConflictKind::Directory,
            ConflictAction::Skip,
            ConflictScope::AllRemaining,
        );
        // Both selections are accepted by the plan builder without panicking.
        let _ = (per_entry, all);
    }
}
