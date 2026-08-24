use crate::StreamBodyAsOptions;
use futures::Stream;
use http_body::Frame;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// How a streamed HTTP body ended, or that it is still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamBodyOutcome {
    /// A periodic snapshot: the body is still being streamed.
    InProgress,
    /// The source stream ended and every frame was handed over.
    Completed,
    /// An error terminated the body; the response is truncated.
    Failed,
    /// The body was dropped before the stream ended, typically because the client
    /// disconnected or a middleware timed the response out.
    Aborted,
}

impl StreamBodyOutcome {
    /// The value used for the `outcome` tracing field.
    pub fn as_str(&self) -> &'static str {
        match self {
            StreamBodyOutcome::InProgress => "in_progress",
            StreamBodyOutcome::Completed => "completed",
            StreamBodyOutcome::Failed => "failed",
            StreamBodyOutcome::Aborted => "aborted",
        }
    }
}

/// A snapshot of how much of a streamed HTTP body has been produced so far.
///
/// `items` counts the objects successfully read from your source stream, so an item that
/// failed is not counted. Note that an item is whatever the format consumes: for the Arrow
/// format that is a `RecordBatch`, not a row.
///
/// `bytes` counts the body bytes actually handed to the HTTP layer, which is why it can be
/// lower than what was serialized: with `buffering_bytes`, bytes still sitting in the buffer
/// when an error occurs are discarded and never counted.
#[derive(Debug, Clone, Copy)]
pub struct StreamProgress {
    pub items: u64,
    pub bytes: u64,
    pub elapsed: Duration,
    pub outcome: StreamBodyOutcome,
}

/// A callback invoked with progress snapshots while streaming an HTTP body.
pub type StreamBodyAsProgressHandler = Arc<dyn Fn(&StreamProgress) + Send + Sync + 'static>;

/// Shared accounting for one streamed body.
///
/// Items are counted on the source stream (the only place items still exist as items) and
/// bytes on the final frame stream, so the two counters live in different combinators and
/// share this state. The ordering is `Relaxed` throughout: these are counters, not
/// synchronisation.
pub(crate) struct StreamProgressState {
    items: AtomicU64,
    bytes: AtomicU64,
    last_emit_micros: AtomicU64,
    next_item_step: AtomicU64,
    finalized: AtomicBool,
    start: Instant,
    interval_micros: Option<u64>,
    item_step: Option<u64>,
    on_progress: Option<StreamBodyAsProgressHandler>,
    #[cfg(feature = "tracing")]
    span: tracing::Span,
}

impl StreamProgressState {
    /// Returns `None` when nobody is listening, in which case the caller leaves the stream
    /// pipeline completely untouched.
    #[cfg_attr(not(feature = "tracing"), allow(unused_variables))]
    pub(crate) fn maybe_new(
        format: Option<&str>,
        options: &StreamBodyAsOptions,
    ) -> Option<Arc<Self>> {
        if options.on_progress.is_none() && !Self::tracing_enabled() {
            return None;
        }

        // A step of zero would never advance, so treat it as "disabled" rather than looping.
        let item_step = options.progress_items.filter(|step| *step > 0);

        let state = Arc::new(Self {
            items: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            last_emit_micros: AtomicU64::new(0),
            next_item_step: AtomicU64::new(item_step.unwrap_or(u64::MAX)),
            finalized: AtomicBool::new(false),
            start: Instant::now(),
            interval_micros: options
                .progress_interval
                .map(|interval| interval.as_micros() as u64),
            item_step,
            on_progress: options.on_progress.clone(),
            #[cfg(feature = "tracing")]
            span: Self::new_span(format, options),
        });

        Some(state)
    }

    /// The span covering the whole body, created here, while the handler's request span is
    /// still the current one, so collectors nest it under the request rather than orphaning
    /// it. The body itself is polled later, from the connection task, where the request span
    /// is no longer current.
    ///
    /// Every counter is declared up front as an empty field so it can be filled in later with
    /// [`tracing::Span::record`]: collectors that read span attributes (OpenTelemetry and
    /// friends) then see `items`/`bytes`/`outcome` as structured values on a span whose
    /// duration is the streaming duration, instead of having to parse log messages.
    #[cfg(feature = "tracing")]
    fn new_span(format: Option<&str>, options: &StreamBodyAsOptions) -> tracing::Span {
        // A `None` format records nothing at all: `Option` is a `Value` that simply skips the
        // field when it is empty.
        let span = tracing::info_span!(
            target: "axum_streams",
            "axum_streams::stream_body",
            format = format,
            buffering_ready_items = tracing::field::Empty,
            buffering_bytes = tracing::field::Empty,
            items = tracing::field::Empty,
            bytes = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty,
            outcome = tracing::field::Empty,
        );

        if let Some(buffering_ready_items) = options.buffering_ready_items {
            span.record("buffering_ready_items", buffering_ready_items as u64);
        }
        if let Some(buffering_bytes) = options.buffering_bytes {
            span.record("buffering_bytes", buffering_bytes as u64);
        }

        span
    }

    /// Checked at `ERROR`, the least verbose level the accounting can produce: a failed body
    /// reports there, so gating any higher would mean `RUST_LOG=axum_streams=error` silently
    /// loses the totals of the very responses it asked about. Every more verbose filter
    /// enables `ERROR` too, so this can never suppress wanted output.
    #[cfg(feature = "tracing")]
    fn tracing_enabled() -> bool {
        tracing::enabled!(target: "axum_streams", tracing::Level::ERROR)
    }

    #[cfg(not(feature = "tracing"))]
    fn tracing_enabled() -> bool {
        false
    }

    pub(crate) fn record_item(&self) {
        self.items.fetch_add(1, Ordering::Relaxed);
    }

    fn record_frame(&self, len: u64) {
        let bytes = self.bytes.fetch_add(len, Ordering::Relaxed) + len;
        let items = self.items.load(Ordering::Relaxed);

        // A stream can outlive its own terminal snapshot: with `buffering_ready_items` an
        // error does not stop the underlying stream, so a consumer that keeps polling past it
        // still gets frames. Nothing may be reported after the summary, or the final snapshot
        // would no longer be final.
        let reported = self.finalized.load(Ordering::Relaxed);

        #[cfg(feature = "tracing")]
        tracing::trace!(
            target: "axum_streams",
            parent: &self.span,
            frame_bytes = len,
            items,
            bytes,
            "Streamed an HTTP body frame"
        );

        if !reported && self.should_emit(items) {
            self.emit(StreamBodyOutcome::InProgress, items, bytes);
        }
    }

    /// The two triggers are OR'd, and emitting resets both, so a frame produces at most one
    /// progress event.
    fn should_emit(&self, items: u64) -> bool {
        let mut emit = false;

        if let Some(step) = self.item_step {
            if items >= self.next_item_step.load(Ordering::Relaxed) {
                // Skip past every step the current count already crossed, so a single frame
                // carrying many items cannot queue up a burst of events.
                self.next_item_step
                    .store(items - (items % step) + step, Ordering::Relaxed);
                emit = true;
            }
        }

        if let Some(interval) = self.interval_micros {
            let elapsed = self.start.elapsed().as_micros() as u64;
            let since_last = elapsed.saturating_sub(self.last_emit_micros.load(Ordering::Relaxed));
            if emit || since_last >= interval {
                self.last_emit_micros.store(elapsed, Ordering::Relaxed);
                emit = true;
            }
        }

        emit
    }

    /// Emits the terminal snapshot, exactly once per body.
    fn finalize(&self, outcome: StreamBodyOutcome) {
        if self.finalized.swap(true, Ordering::Relaxed) {
            return;
        }

        let items = self.items.load(Ordering::Relaxed);
        let bytes = self.bytes.load(Ordering::Relaxed);

        // Recorded once, here rather than on every progress report: subscribers are free to
        // treat `record` as append-only (`tracing-subscriber`'s formatter does), so writing a
        // field repeatedly makes the rendered span grow with every tick. Once per span also
        // means the values a collector reads are the final ones.
        #[cfg(feature = "tracing")]
        {
            self.span.record("items", items);
            self.span.record("bytes", bytes);
            self.span
                .record("elapsed_ms", self.start.elapsed().as_millis() as u64);
            self.span.record("outcome", outcome.as_str());
        }

        self.emit(outcome, items, bytes);
    }

    fn emit(&self, outcome: StreamBodyOutcome, items: u64, bytes: u64) {
        let progress = StreamProgress {
            items,
            bytes,
            elapsed: self.start.elapsed(),
            outcome,
        };

        #[cfg(feature = "tracing")]
        {
            let elapsed_ms = progress.elapsed.as_millis() as u64;

            match outcome {
                // Interim progress is chatter; the summary is the line worth keeping, and a
                // truncated response is worth an operator's attention.
                StreamBodyOutcome::InProgress => tracing::debug!(
                    target: "axum_streams",
                    parent: &self.span,
                    items,
                    bytes,
                    elapsed_ms,
                    "Streaming an HTTP body"
                ),
                StreamBodyOutcome::Failed => tracing::error!(
                    target: "axum_streams",
                    parent: &self.span,
                    items,
                    bytes,
                    elapsed_ms,
                    outcome = outcome.as_str(),
                    "Failed streaming an HTTP body"
                ),
                // Completed, and aborted: a client going away mid-stream is ordinary.
                _ => tracing::info!(
                    target: "axum_streams",
                    parent: &self.span,
                    items,
                    bytes,
                    elapsed_ms,
                    outcome = outcome.as_str(),
                    "Finished streaming an HTTP body"
                ),
            }
        }

        if let Some(handler) = &self.on_progress {
            handler(&progress);
        }
    }
}

/// Counts the bytes of the final frame stream and owns the outcome state machine.
///
/// It wraps the outermost stream on purpose: that way `bytes` is what actually reached the
/// HTTP layer rather than what was buffered, and its `Drop` is the body's drop, which is the
/// only way to notice a client that hung up mid-stream.
pub(crate) struct ProgressStream<S> {
    inner: S,
    state: Arc<StreamProgressState>,
}

impl<S> ProgressStream<S> {
    pub(crate) fn new(inner: S, state: Arc<StreamProgressState>) -> Self {
        Self { inner, state }
    }
}

impl<S> Stream for ProgressStream<S>
where
    S: Stream<Item = Result<Frame<axum::body::Bytes>, axum::Error>> + Unpin,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Polling here drives the whole pipeline synchronously, the user's own stream
        // included, so entering the span gives everything it logs the body's context.
        #[cfg(feature = "tracing")]
        let _entered = this.state.span.enter();

        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                // Always a data frame here, but `data_ref` cannot know that.
                let len = frame.data_ref().map_or(0, |data| data.len() as u64);
                this.state.record_frame(len);
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(err))) => {
                // The error itself is already reported by `StreamBodyAs::report_errors`; this
                // adds the totals it was truncated at.
                this.state.finalize(StreamBodyOutcome::Failed);
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(None) => {
                this.state.finalize(StreamBodyOutcome::Completed);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for ProgressStream<S> {
    fn drop(&mut self) {
        // A no-op when the stream already ran to completion or failed.
        self.state.finalize(StreamBodyOutcome::Aborted);
    }
}
