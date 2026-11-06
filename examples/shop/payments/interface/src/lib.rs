//! Interface layer of the `payments` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Payments` implementation
//! and exposes it. It never sees `domain` (ADR 0003). A scaffolded module would
//! let `#[seiva::controller]` generate the axum routes from the facade; this
//! example keeps a plain wrapper so the layering is provable without the
//! transport runtime.

use shop_payments_facade::{Charge, PaymentDto, PaymentId, Payments};

/// Wraps a `Payments` implementation and exposes it.
pub struct PaymentsApi<S: Payments> {
    pub service: S,
}

impl<S: Payments> PaymentsApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /v1/payments/charges` route will call.
    pub async fn charge(&self, order_id: u128, amount_cents: u64) -> Result<PaymentDto, String> {
        self.service
            .charge(Charge {
                order_id,
                amount_cents,
            })
            .await
    }

    /// The handler the `GET /v1/payments/{id}` route will call.
    pub async fn get(&self, id: u128) -> Result<PaymentDto, String> {
        self.service.get(PaymentId { id }).await
    }
}
