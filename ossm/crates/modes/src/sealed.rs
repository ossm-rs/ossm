//! Sealing for traits only this crate implements.

/// Implemented only by this crate.
///
/// Traits that take it as a supertrait keep their implementations here,
/// so no other crate can add capabilities or supervisor plumbing:
/// [`Extract`](crate::Extract) and the other traits this crate seals use
/// `Sealed`, and [`Mode`](crate::Mode) uses `Sealed<fn() -> Args>`, once
/// per argument list, since every async fn of extractors is a mode.
pub trait Sealed<Marker = ()> {}
