//! A request body that fails when its client stops sending: the clock runs only while the handler waits for the next
//! frame and is cleared when one arrives, so a handler busy elsewhere before it reads is never charged for it.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody};
use http_body::{Frame, SizeHint};
use tokio::time::Sleep;

/// The body's next frame did not arrive within the idle limit.
#[derive(Debug, thiserror::Error)]
#[error("the request body stopped arriving")]
pub(crate) struct Stalled;

/// `inner`, failing with [`Stalled`] once a wait for its next frame lasts `idle`.
pub(crate) struct IdleBody {
    inner: Body,
    idle: Duration,
    waiting: Option<Pin<Box<Sleep>>>,
}

impl IdleBody {
    pub(crate) const fn new(inner: Body, idle: Duration) -> Self {
        Self {
            inner,
            idle,
            waiting: None,
        }
    }
}

impl HttpBody for IdleBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(frame) => {
                this.waiting = None;
                Poll::Ready(frame)
            }
            Poll::Pending => {
                let idle = this.idle;
                let waiting = this
                    .waiting
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(idle)));
                match waiting.as_mut().poll(cx) {
                    Poll::Ready(()) => Poll::Ready(Some(Err(axum::Error::new(Stalled)))),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
