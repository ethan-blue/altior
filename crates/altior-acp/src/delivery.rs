//! Prompt delivery classification onto the frozen
//! [`altior_domain::DeliveryState`] vocabulary (ADR 0004, ADR 0007).
//!
//! One [`PromptDelivery`] tracks one prompt attempt:
//!
//! - starts [`Absent`] — provably not delivered;
//! - a failed write (dead process, encode failure) keeps it [`Absent`];
//! - a successful write moves it to [`Indeterminate`] — bytes sent, no
//!   proof of receipt;
//! - an error response names it [`Rejected`];
//! - a prompt response (any stop reason, including `cancelled`) confirms
//!   it [`Confirmed`];
//! - a crash, stream end, or idle timeout while outstanding pins it
//!   [`Indeterminate`] — never `Absent`, because the bytes may have been
//!   consumed.
//!
//! Only [`Absent`] and [`Rejected`] may be re-sent; the tracker enforces
//! the rule that ADR 0002 states and AGENTS.md inherits.
//!
//! [`Absent`]: altior_domain::DeliveryState::Absent
//! [`Indeterminate`]: altior_domain::DeliveryState::Indeterminate
//! [`Rejected`]: altior_domain::DeliveryState::Rejected
//! [`Confirmed`]: altior_domain::DeliveryState::Confirmed

use altior_domain::DeliveryState;

use crate::error::AcpError;

/// Why an outstanding delivery became indeterminate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryCause {
    /// The agent process exited or its stream ended.
    ProcessExited,
    /// The idle budget elapsed with no response.
    IdleTimeout,
}

/// The delivery state of one prompt attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptDelivery {
    state: DeliveryState,
    outstanding: bool,
    cause: Option<DeliveryCause>,
}

impl PromptDelivery {
    /// A prompt that provably has not been delivered yet.
    #[must_use]
    pub const fn not_sent() -> Self {
        Self {
            state: DeliveryState::Absent,
            outstanding: false,
            cause: None,
        }
    }

    /// The current classification.
    #[must_use]
    pub const fn state(self) -> DeliveryState {
        self.state
    }

    /// Why the attempt ended indeterminate, when it did.
    #[must_use]
    pub const fn indeterminate_cause(self) -> Option<DeliveryCause> {
        self.cause
    }

    /// Whether a response is still owed for this attempt.
    #[must_use]
    pub const fn outstanding(self) -> bool {
        self.outstanding
    }

    /// Whether the prompt may be re-sent: only a provably [`Absent`] or
    /// explicitly [`Rejected`] attempt qualifies (ADR 0002).
    ///
    /// [`Absent`]: altior_domain::DeliveryState::Absent
    /// [`Rejected`]: altior_domain::DeliveryState::Rejected
    #[must_use]
    pub const fn may_resend(self) -> bool {
        matches!(self.state, DeliveryState::Absent | DeliveryState::Rejected)
    }

    /// Reports that the prompt line entered the pipe. Delivery is now
    /// indeterminate until a response or failure classifies it.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when the attempt already has a
    /// verdict or is already outstanding.
    pub fn mark_written(&mut self) -> Result<(), AcpError> {
        if self.outstanding {
            return Err(AcpError::OutOfOrder {
                attempted: "mark a prompt written twice for one attempt",
            });
        }
        self.outstanding = true;
        self.state = DeliveryState::Indeterminate;
        Ok(())
    }

    /// Reports that the write failed before any byte was consumed. The
    /// attempt stays provably [`Absent`] and may be re-sent.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when the attempt is already
    /// outstanding or has a verdict.
    pub fn mark_write_failed(&mut self) -> Result<(), AcpError> {
        if self.outstanding {
            return Err(AcpError::OutOfOrder {
                attempted: "report a failed write after a successful one",
            });
        }
        Ok(())
    }

    /// Reports that the agent answered the prompt with a result. Any stop
    /// reason — including `cancelled` — proves receipt.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when no attempt is outstanding.
    pub fn on_prompt_response(&mut self) -> Result<DeliveryState, AcpError> {
        self.settle(DeliveryState::Confirmed)
    }

    /// Reports that the agent answered the prompt with a JSON-RPC error.
    /// The prompt was received and refused.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when no attempt is outstanding.
    pub fn on_error_response(&mut self) -> Result<DeliveryState, AcpError> {
        self.settle(DeliveryState::Rejected)
    }

    /// Reports that the connection died while the attempt was
    /// outstanding. The classification is [`Indeterminate`]; a re-send is
    /// forbidden.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::OutOfOrder`] when no attempt is outstanding.
    pub fn on_connection_lost(&mut self, cause: DeliveryCause) -> Result<DeliveryState, AcpError> {
        if !self.outstanding {
            return Err(AcpError::OutOfOrder {
                attempted: "settle a prompt delivery with nothing outstanding",
            });
        }
        self.cause = Some(cause);
        self.settle(DeliveryState::Indeterminate)
    }

    fn settle(&mut self, verdict: DeliveryState) -> Result<DeliveryState, AcpError> {
        if !self.outstanding {
            return Err(AcpError::OutOfOrder {
                attempted: "settle a prompt delivery with nothing outstanding",
            });
        }
        self.outstanding = false;
        self.state = verdict;
        Ok(verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwritten_prompts_are_absent_and_resendable() {
        let mut delivery = PromptDelivery::not_sent();
        assert_eq!(delivery.state(), DeliveryState::Absent);
        assert!(delivery.may_resend());
        delivery.mark_write_failed().unwrap();
        assert_eq!(delivery.state(), DeliveryState::Absent);
        assert!(delivery.may_resend());
    }

    #[test]
    fn responses_confirm_and_reject() {
        let mut confirmed = PromptDelivery::not_sent();
        confirmed.mark_written().unwrap();
        assert!(!confirmed.may_resend());
        assert_eq!(
            confirmed.on_prompt_response().unwrap(),
            DeliveryState::Confirmed
        );
        assert!(!confirmed.may_resend());

        let mut rejected = PromptDelivery::not_sent();
        rejected.mark_written().unwrap();
        assert_eq!(
            rejected.on_error_response().unwrap(),
            DeliveryState::Rejected
        );
        assert!(rejected.may_resend());
    }

    #[test]
    fn crashes_and_timeouts_are_never_absent() {
        for cause in [DeliveryCause::ProcessExited, DeliveryCause::IdleTimeout] {
            let mut delivery = PromptDelivery::not_sent();
            delivery.mark_written().unwrap();
            assert_eq!(
                delivery.on_connection_lost(cause).unwrap(),
                DeliveryState::Indeterminate
            );
            assert!(!delivery.may_resend(), "{cause:?} must never auto-resend");
        }
    }

    #[test]
    fn reports_are_order_sensitive() {
        let mut delivery = PromptDelivery::not_sent();
        assert!(matches!(
            delivery.on_prompt_response(),
            Err(AcpError::OutOfOrder { .. })
        ));
        delivery.mark_written().unwrap();
        assert!(matches!(
            delivery.mark_written(),
            Err(AcpError::OutOfOrder { .. })
        ));
        delivery.on_prompt_response().unwrap();
        assert!(matches!(
            delivery.on_prompt_response(),
            Err(AcpError::OutOfOrder { .. })
        ));
    }
}
