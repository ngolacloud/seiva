//! Shared value types across modules. No business rules live here.

/// A tenant identifier, shared by every module's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub u128);
