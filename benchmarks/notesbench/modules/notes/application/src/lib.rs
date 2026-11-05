//! Application layer: the service implementing the facade over an outbound
//! repository port and the framework's cross-cutting `seiva-ports`.
//!
//! Depends on `domain`, this module's `facade` (DTOs + trait) and
//! `seiva-ports`. It names NO transport, framework-runtime or DORM item —
//! the HTTP coupling lives in `interface`, the persistence in
//! `infrastructure`. This is what keeps the use case independently testable.
#![allow(async_fn_in_trait)]

use std::future::Future;

use notes_domain::Record;
use notes_facade::{CreateRecord, RecordDto, RecordError, Notes};
use seiva_ports::{BoxError, IdGenerator, PortResult};

/// Outbound port: persistence of records in the domain's language.
/// Implemented in `infrastructure` (in-memory by default, DORM-backed via
/// the `dorm` feature). The future is `Send` so a use case can `.await` it
/// on a multi-threaded runtime.
pub trait RecordRepository {
    /// Persists a record.
    fn save(&self, record: &Record) -> impl Future<Output = PortResult<()>> + Send;
}

/// The application service: it implements the [`Notes`] facade by running
/// the use case and mapping the domain entity to the DTO. Generic over the
/// repository port and an [`IdGenerator`] so it unit-tests with fakes. It is
/// `Clone` (when its ports are) so the transport layer can hold it as state.
#[derive(Clone)]
pub struct RecordService<R, G> {
    /// The outbound persistence port.
    pub repository: R,
    /// The framework id generator.
    pub ids: G,
}

impl<R, G> RecordService<R, G>
where
    R: RecordRepository + Sync,
    G: IdGenerator,
{
    async fn create_record(&self, cmd: CreateRecord) -> PortResult<RecordDto> {
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let record = Record::new(id, cmd.title).map_err(BoxError::from)?;
        self.repository.save(&record).await?;
        Ok(RecordDto {
            id: record.id,
            title: record.title,
        })
    }
}

impl<R, G> Notes for RecordService<R, G>
where
    R: RecordRepository + Sync,
    G: IdGenerator,
{
    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError> {
        // Map the internal error to the contract's error type: no internal
        // detail leaks across the module boundary beyond a message.
        self.create_record(cmd)
            .await
            .map_err(|err| RecordError(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_ports::Id;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        saved: Mutex<Vec<Record>>,
    }

    impl RecordRepository for FakeRepo {
        async fn save(&self, record: &Record) -> PortResult<()> {
            self.saved.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    struct FixedIds;
    impl IdGenerator for FixedIds {
        fn new_id(&self) -> Id {
            Id([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])
        }
    }

    #[tokio::test]
    async fn create_persists_and_returns_the_dto() {
        let service = RecordService {
            repository: FakeRepo::default(),
            ids: FixedIds,
        };
        let dto = service
            .create(CreateRecord { title: "hello".into() })
            .await
            .unwrap();
        assert_eq!(dto.id, 1);
        assert_eq!(dto.title, "hello");
        assert_eq!(service.repository.saved.lock().unwrap().len(), 1);
    }
}
