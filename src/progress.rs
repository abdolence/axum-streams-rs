//! Accounting types for a streamed body.
//!
//! These now live in [`http_streams_core`] and are shared with the client-side crate, which
//! had grown an identical implementation independently. They are re-exported here under the
//! names they have always had, so existing code is unaffected.
//!
//! One change is visible: [`StreamProgress`] gained an `errors` counter and is now
//! `#[non_exhaustive]`. Only exhaustive destructuring in an `on_progress` callback is affected;
//! add `..` to such a pattern.

pub use http_streams_core::{
    StreamOutcome as StreamBodyOutcome, StreamProgress,
    StreamProgressHandler as StreamBodyAsProgressHandler,
};
