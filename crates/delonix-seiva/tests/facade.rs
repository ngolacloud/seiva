//! Proves `#[seiva::facade]` generates the contract descriptor (ADR 0001).

use delonix_seiva::contract::Facade;
use delonix_seiva::facade;

// A module's public contract, written the way a real `notes` module would.
// `Ctx` and the DTOs stand in for real types; the macro reads only the
// signatures and the `#[requires(..)]` permission, so these can be aliases.
type Ctx = ();
type NoteDto = u64;
type CreateNote = u64;
type NoteId = u64;
type Result<T> = std::result::Result<T, ()>;

#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    #[policy(timeout = "2s")]
    async fn create(&self, ctx: &Ctx, cmd: CreateNote) -> Result<NoteDto>;

    #[requires("notes.read")]
    #[cached(ttl = "5m")]
    async fn get(&self, ctx: &Ctx, id: NoteId) -> Result<NoteDto>;

    // A method with no permission: the descriptor records an empty `requires`.
    async fn ping(&self) -> Result<()>;
}

#[test]
fn descriptor_captures_name_version_and_methods() {
    let d = NotesFacade::DESCRIPTOR;
    assert_eq!(d.name, "Notes");
    assert_eq!(d.version, "v1");
    assert_eq!(d.methods.len(), 3);

    assert_eq!(d.methods[0].name, "create");
    assert_eq!(d.methods[0].requires, "notes.create");
    assert!(d.methods[0].is_guarded());

    assert_eq!(d.methods[1].name, "get");
    assert_eq!(d.methods[1].requires, "notes.read");

    assert_eq!(d.methods[2].name, "ping");
    assert_eq!(d.methods[2].requires, "");
    assert!(!d.methods[2].is_guarded());
}

#[test]
fn descriptor_is_queryable_by_method_name() {
    let d = NotesFacade::DESCRIPTOR;
    assert_eq!(d.method("get").unwrap().requires, "notes.read");
    assert!(d.method("missing").is_none());
}

// The trait itself survives the macro and is implementable: the helper
// attributes (`#[requires]`, `#[policy]`, `#[cached]`) were consumed, not left
// behind to break compilation.
struct FakeNotes;

impl Notes for FakeNotes {
    async fn create(&self, _ctx: &Ctx, cmd: CreateNote) -> Result<NoteDto> {
        Ok(cmd)
    }
    async fn get(&self, _ctx: &Ctx, _id: NoteId) -> Result<NoteDto> {
        Ok(0)
    }
    async fn ping(&self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn the_trait_is_still_usable() {
    // Drive one method to prove the re-emitted trait compiles and runs.
    let got = futures_lite_block_on(FakeNotes.create(&(), 42));
    assert_eq!(got, Ok(42));
}

// A trivial block_on for a future that completes immediately — avoids adding a
// runtime just to drive one ready future in this test.
fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::pin::pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    fn no_op(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };

    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {
            return output;
        }
    }
}
