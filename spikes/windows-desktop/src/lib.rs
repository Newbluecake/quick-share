#![forbid(unsafe_code)]
//! Disposable primitives for the Windows desktop feasibility probe.

use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

/// Single-flight guard used to prevent overlapping modal workflows.
#[derive(Debug, Default)]
pub struct UiGate {
    active: AtomicBool,
}

impl UiGate {
    /// Acquires the only modal slot, or reports a deterministic busy result.
    pub fn try_acquire(&self) -> Result<UiLease<'_>, UiGateError> {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| UiLease { gate: self })
            .map_err(|_| UiGateError::Busy)
    }

    /// Reports whether a workflow currently owns the modal slot.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

/// RAII ownership of the modal slot.
#[derive(Debug)]
pub struct UiLease<'a> {
    gate: &'a UiGate,
}

impl Drop for UiLease<'_> {
    fn drop(&mut self) {
        self.gate.active.store(false, Ordering::Release);
    }
}

/// Bounded UI admission result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiGateError {
    /// Another modal workflow is active.
    #[error("the desktop UI is busy")]
    Busy,
}

#[cfg(test)]
mod tests {
    use super::{UiGate, UiGateError};

    #[test]
    fn gate_rejects_overlap_and_releases_on_drop() {
        // Arrange
        let gate = UiGate::default();

        // Act
        let first = gate.try_acquire().expect("first workflow acquires UI");
        let overlap = gate.try_acquire();

        // Assert
        assert!(gate.is_active());
        assert!(matches!(overlap, Err(UiGateError::Busy)));
        drop(first);
        assert!(!gate.is_active());
        assert!(gate.try_acquire().is_ok());
    }
}
