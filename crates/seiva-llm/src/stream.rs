//! The streamed-answer type.

use std::pin::Pin;

use futures_core::Stream;

use crate::{LlmError, StreamEvent};

/// A streamed answer: deltas, then always [`StreamEvent::Completed`] with the
/// whole answer — or an error, after which the stream ends.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>;
