//! Transfer-scoped receiver routing for desktop agents with no default output root.

use crate::{
    TransferStore,
    offer::{AuthorizationPermission, AuthorizationToken, OfferManager},
    receiver::{
        ReceiverEndpoint, ReceiverError, ReceiverPolicy, ReceiverProgressEvent, ReceiverService,
    },
};
use quick_share_core::{
    config::ConflictPolicy,
    destination::DestinationPlan,
    receive::{ReceiveBinding, ReceiveBindingStore, ReceiveStateError},
};
use quick_share_protocol::{
    AuthorizationProof, ChunkAck, ChunkData, DeviceId, RequestId, TransferCancel, TransferComplete,
    TransferCompleteAck, TransferId, TransferOffer, TransferStatusRequest, TransferStatusResponse,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};
use thiserror::Error;
use tokio::sync::mpsc;

/// Agent receiver that resolves every transfer through a durable binding.
pub struct ReceiverRouter {
    offers: Arc<OfferManager>,
    bindings: ReceiveBindingStore,
    policy: ReceiverPolicy,
    progress: Option<mpsc::Sender<ReceiverProgressEvent>>,
    state: Mutex<RouterState>,
}

impl std::fmt::Debug for ReceiverRouter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReceiverRouter")
            .field("bindings", &self.bindings)
            .field("policy", &self.policy)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

impl ReceiverRouter {
    pub fn new(
        offers: Arc<OfferManager>,
        bindings: ReceiveBindingStore,
        policy: ReceiverPolicy,
        progress: Option<mpsc::Sender<ReceiverProgressEvent>>,
    ) -> Result<Self, ReceiverRouterError> {
        policy.validate()?;
        Ok(Self {
            offers,
            bindings,
            policy,
            progress,
            state: Mutex::new(RouterState::default()),
        })
    }

    /// Persists routing state before the caller grants upload authorization.
    pub fn bind(
        &self,
        binding: ReceiveBinding,
        offer: &TransferOffer,
    ) -> Result<(), ReceiverRouterError> {
        validate_binding(&binding, offer)?;
        self.bindings.bind(binding)?;
        Ok(())
    }

    /// Atomically replaces a conflict plan without changing the bound root or peer.
    pub fn update_plan(
        &self,
        transfer_id: TransferId,
        offer: &TransferOffer,
        plan: DestinationPlan,
    ) -> Result<ReceiveBinding, ReceiverRouterError> {
        let existing = self.binding(transfer_id)?;
        let updated = ReceiveBinding {
            destination_plan: plan,
            ..existing
        };
        validate_binding(&updated, offer)?;
        Ok(self
            .bindings
            .update_plan(transfer_id, updated.destination_plan)?)
    }

    pub fn status(
        &self,
        peer: &DeviceId,
        request: TransferStatusRequest,
        now: Instant,
    ) -> Result<TransferStatusResponse, ReceiverRouterError> {
        let binding = self.authorized_binding(
            &request.authorization,
            peer,
            request.transfer_id,
            AuthorizationPermission::TransferStatus,
            now,
        )?;
        self.service_for(&binding)?
            .status(peer, request, now)
            .map_err(Into::into)
    }

    pub fn receive_chunk(
        &self,
        peer: &DeviceId,
        request_id: RequestId,
        frame: ChunkData,
        now: Instant,
    ) -> Result<Option<ChunkAck>, ReceiverRouterError> {
        let transfer_id = frame.descriptor.transfer_id;
        let binding = self.authorized_binding(
            &frame.authorization,
            peer,
            transfer_id,
            AuthorizationPermission::ChunkUpload,
            now,
        )?;
        let upload_key = (peer.clone(), request_id, transfer_id);
        {
            let mut state = self.lock_state()?;
            if frame.fragment_offset == 0 && !state.uploads.contains(&upload_key) {
                if state.uploads.len() >= self.policy.max_file_streams {
                    return Err(ReceiverRouterError::FileStreamLimit);
                }
                state.uploads.insert(upload_key.clone());
            }
        }
        let final_fragment = frame.final_fragment;
        let result = match self.service_for(&binding) {
            Ok(service) => service
                .receive_chunk(peer, request_id, frame, now)
                .map_err(ReceiverRouterError::from),
            Err(error) => Err(error),
        };
        if final_fragment || result.is_err() {
            self.lock_state()?.uploads.remove(&upload_key);
        }
        result
    }

    pub fn complete(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
    ) -> Result<TransferCompleteAck, ReceiverRouterError> {
        let binding = self.authorized_binding(
            &request.authorization,
            peer,
            request.transfer_id,
            AuthorizationPermission::Complete,
            now,
        )?;
        self.service_for(&binding)?
            .complete_with_plan(peer, request, now, &binding.destination_plan)
            .map_err(Into::into)
    }

    pub fn cancel(
        &self,
        peer: &DeviceId,
        request: TransferCancel,
        now: Instant,
    ) -> Result<(), ReceiverRouterError> {
        let binding = self.authorized_binding(
            &request.authorization,
            peer,
            request.transfer_id,
            AuthorizationPermission::Cancel,
            now,
        )?;
        self.service_for(&binding)?
            .cancel(peer, request, now)
            .map_err(Into::into)
    }

    pub fn disconnect(&self, peer: &DeviceId) -> Result<usize, ReceiverRouterError> {
        self.lock_state()?
            .uploads
            .retain(|(owner, _, _)| owner != peer);
        let services = self.services()?;
        services.into_iter().try_fold(0_usize, |count, service| {
            service
                .disconnect(peer)
                .map(|released| count + released)
                .map_err(ReceiverRouterError::from)
        })
    }

    /// Pauses active transfers but deliberately retains bindings and staging for resume.
    pub fn pause_all(&self) -> Result<usize, ReceiverRouterError> {
        self.services()?
            .into_iter()
            .try_fold(0_usize, |count, service| {
                service
                    .pause_all()
                    .map(|paused| count + paused)
                    .map_err(ReceiverRouterError::from)
            })
    }

    /// Deletes staging first and removes the active binding only after cleanup succeeds.
    pub fn cleanup(&self, transfer_id: TransferId) -> Result<(), ReceiverRouterError> {
        let binding = self.binding(transfer_id)?;
        let service = {
            let mut state = self.lock_state()?;
            state
                .uploads
                .retain(|(_, _, active_transfer)| *active_transfer != transfer_id);
            state.services.remove(&transfer_id)
        };
        if let Some(service) = service {
            service.cleanup(transfer_id)?;
        } else {
            TransferStore::discard_staging(&binding.output_root, transfer_id)?;
        }
        self.bindings.remove(transfer_id)?;
        Ok(())
    }

    fn authorized_binding(
        &self,
        proof: &AuthorizationProof,
        peer: &DeviceId,
        transfer_id: TransferId,
        permission: AuthorizationPermission,
        now: Instant,
    ) -> Result<ReceiveBinding, ReceiverRouterError> {
        let token = proof.with_bytes(|bytes| AuthorizationToken::from_bytes(*bytes));
        let authorized = self
            .offers
            .authorize(&token, peer, transfer_id, permission, now)?;
        let binding = self.binding(transfer_id)?;
        if binding.sender_device_id != *peer
            || binding.manifest_digest != authorized.manifest_digest
            || !binding.output_root.is_dir()
        {
            return Err(ReceiverRouterError::BindingMismatch);
        }
        binding
            .destination_plan
            .validate_for_offer(&authorized.offer)?;
        Ok(binding)
    }

    fn binding(&self, transfer_id: TransferId) -> Result<ReceiveBinding, ReceiverRouterError> {
        match self.bindings.get(transfer_id) {
            Ok(Some(binding)) => Ok(binding),
            Ok(None) => Err(ReceiverRouterError::BindingNotFound),
            Err(ReceiveStateError::InvalidDestination(_)) => {
                Err(ReceiverRouterError::BindingMismatch)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn service_for(
        &self,
        binding: &ReceiveBinding,
    ) -> Result<Arc<ReceiverService>, ReceiverRouterError> {
        let mut state = self.lock_state()?;
        if let Some(service) = state.services.get(&binding.transfer_id) {
            return Ok(Arc::clone(service));
        }
        let active = state
            .services
            .iter()
            .filter(|(transfer_id, service)| service.is_active(**transfer_id))
            .count();
        if active >= self.policy.max_receive_tasks {
            return Err(ReceiverRouterError::ReceiveTaskLimit);
        }
        let service = Arc::new(ReceiverService::new_with_progress(
            Arc::clone(&self.offers),
            &binding.output_root,
            ReceiverPolicy {
                conflict: ConflictPolicy::Error,
                max_receive_tasks: 1,
                max_file_streams: self.policy.max_file_streams,
            },
            self.progress.clone(),
        )?);
        state
            .services
            .insert(binding.transfer_id, Arc::clone(&service));
        Ok(service)
    }

    fn services(&self) -> Result<Vec<Arc<ReceiverService>>, ReceiverRouterError> {
        Ok(self
            .lock_state()?
            .services
            .values()
            .map(Arc::clone)
            .collect())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, RouterState>, ReceiverRouterError> {
        self.state.lock().map_err(|_| ReceiverRouterError::Internal)
    }
}

impl ReceiverEndpoint for ReceiverRouter {
    fn status(
        &self,
        peer: &DeviceId,
        request: TransferStatusRequest,
        now: Instant,
    ) -> Result<TransferStatusResponse, ReceiverError> {
        ReceiverRouter::status(self, peer, request, now).map_err(map_endpoint_error)
    }

    fn receive_chunk(
        &self,
        peer: &DeviceId,
        request_id: RequestId,
        frame: ChunkData,
        now: Instant,
    ) -> Result<Option<ChunkAck>, ReceiverError> {
        ReceiverRouter::receive_chunk(self, peer, request_id, frame, now)
            .map_err(map_endpoint_error)
    }

    fn complete(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
    ) -> Result<TransferCompleteAck, ReceiverError> {
        ReceiverRouter::complete(self, peer, request, now).map_err(map_endpoint_error)
    }

    fn cancel(
        &self,
        peer: &DeviceId,
        request: TransferCancel,
        now: Instant,
    ) -> Result<(), ReceiverError> {
        ReceiverRouter::cancel(self, peer, request, now).map_err(map_endpoint_error)
    }

    fn disconnect(&self, peer: &DeviceId) -> Result<usize, ReceiverError> {
        ReceiverRouter::disconnect(self, peer).map_err(map_endpoint_error)
    }
}

fn map_endpoint_error(error: ReceiverRouterError) -> ReceiverError {
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

#[derive(Default)]
struct RouterState {
    services: BTreeMap<TransferId, Arc<ReceiverService>>,
    uploads: BTreeSet<(DeviceId, RequestId, TransferId)>,
}

fn validate_binding(
    binding: &ReceiveBinding,
    offer: &TransferOffer,
) -> Result<(), ReceiverRouterError> {
    let digest = *blake3::hash(
        &serde_json::to_vec(offer)
            .map_err(|error| ReceiverRouterError::InvalidOffer(error.to_string()))?,
    )
    .as_bytes();
    if binding.transfer_id != offer.transfer_id
        || binding.sender_device_id != offer.sender.device_id
        || binding.manifest_digest != digest
        || !binding.output_root.is_dir()
    {
        return Err(ReceiverRouterError::BindingMismatch);
    }
    binding.destination_plan.validate_for_offer(offer)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum ReceiverRouterError {
    #[error("receive binding was not found")]
    BindingNotFound,
    #[error("receive binding does not match the authenticated offer or output root")]
    BindingMismatch,
    #[error("receiver transfer-task limit reached")]
    ReceiveTaskLimit,
    #[error("receiver file-stream limit reached")]
    FileStreamLimit,
    #[error("invalid transfer offer: {0}")]
    InvalidOffer(String),
    #[error("receiver router state is unavailable")]
    Internal,
    #[error(transparent)]
    Receiver(#[from] ReceiverError),
    #[error(transparent)]
    Offer(#[from] crate::offer::OfferError),
    #[error(transparent)]
    ReceiveState(#[from] ReceiveStateError),
    #[error(transparent)]
    DestinationPlan(#[from] quick_share_core::destination::DestinationPlanError),
    #[error(transparent)]
    Store(#[from] crate::StoreError),
}
