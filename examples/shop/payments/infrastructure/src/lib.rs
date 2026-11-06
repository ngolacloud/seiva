//! Infrastructure layer of the `payments` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In-memory here so the
//! wiring is provable without a database; the real module is DORM-backed.

use seiva_ports::PortResult;
use shop_payments_application::PaymentRepository;
use shop_payments_domain::Payment;
use std::sync::Mutex;

/// An in-memory [`PaymentRepository`]. Replace with the DORM-backed adapter.
#[derive(Default)]
pub struct InMemoryPaymentRepository {
    payments: Mutex<Vec<Payment>>,
}

impl InMemoryPaymentRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every payment stored so far — for composition-level assertions.
    pub fn payments(&self) -> Vec<Payment> {
        self.payments.lock().expect("repository poisoned").clone()
    }
}

impl PaymentRepository for InMemoryPaymentRepository {
    async fn save_payment(&self, payment: &Payment) -> PortResult<()> {
        self.payments
            .lock()
            .map_err(|_| "repository lock poisoned")?
            .push(payment.clone());
        Ok(())
    }

    async fn get_payment(&self, id: u128) -> PortResult<Option<Payment>> {
        Ok(self
            .payments
            .lock()
            .map_err(|_| "repository lock poisoned")?
            .iter()
            .find(|p| p.id == id)
            .cloned())
    }
}
