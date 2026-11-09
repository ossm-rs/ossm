#![no_std]

extern crate alloc;

mod ctx;
mod extract;
mod input;
mod mode;
mod ossm_mode;
mod remote;
mod sealed;
mod supervisor;

use ctx::ModeCtx;

pub use extract::{Extract, Input, Motion, MotionStatus, Patterns};
pub use input::{InputKind, InputKinds, Message, PatternInput, Received};
pub use mode::Mode;
pub use ossm_mode::{OssmMode, patterns};
pub use remote::{Rejected, RemoteHandle, RemoteId, Remotes, Request, Scope, Status};
pub use supervisor::{Entry, ModeKey, ModeList, ModeRunner, Modes};
