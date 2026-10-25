use std::time::Duration;
use std::{
    cell::{Cell, RefCell},
    future::{Future, poll_fn},
    pin::pin,
    rc::{Rc, Weak},
    task::{Poll, Waker},
};

use df_rpc_bridge::CONCURRENT_STREAMS;
use tonic::{Code, Response, Status, Streaming, metadata::MetadataMap};

/// Observed local RPC lifetime; none of these facts confirms a game decision.
/// The original operation receipt/lookup owner remains independent of connectivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    Waiting,
    ResponseReceived,
    StreamFinished,
    GrpcStatus(Code),
    Cancelled,
    ConnectionClosed,
    WaitDisposed,
}

/// Opaque local scope identity. This is neither a binding nor a credential.
/// Tokens remain distinct after reconnect; closed scopes never become active again.
#[derive(Clone)]
pub struct ConnectionGeneration {
    closed: Rc<Cell<bool>>,
}

impl ConnectionGeneration {
    pub fn is_active(&self) -> bool {
        !self.closed.get()
    }

    pub fn same_scope(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.closed, &other.closed)
    }
}

/// Local lifetime failures remain distinct from the server's unmodified gRPC status.
/// Cancellation stops this wait/stream; it cannot undo an accepted server mutation.
#[derive(Debug)]
pub enum RpcError {
    Status(Status),
    Cancelled,
    ConnectionClosed,
    Capacity,
    StreamClosed,
    StreamNotDrained,
}

/// Capped exponential retry window; the caller supplies a uniform sample for full jitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectBackoff {
    base_delay: Duration,
    maximum_delay: Duration,
    retry_index: u32,
}

const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(250);
const RECONNECT_MAXIMUM_DELAY: Duration = Duration::from_secs(8);

struct ReconnectOwner {
    backoff: Cell<ReconnectBackoff>,
    intentionally_closed: Cell<bool>,
}

impl ReconnectOwner {
    fn new() -> Self {
        Self {
            backoff: Cell::new(ReconnectBackoff {
                base_delay: RECONNECT_BASE_DELAY,
                maximum_delay: RECONNECT_MAXIMUM_DELAY,
                retry_index: 0,
            }),
            intentionally_closed: Cell::new(false),
        }
    }

    fn schedule_after_disconnect(&self, jitter_sample: Duration) -> Option<Duration> {
        if self.intentionally_closed.get() {
            return None;
        }
        let mut backoff = self.backoff.get();
        let delay = backoff.delay(jitter_sample);
        backoff.advance();
        self.backoff.set(backoff);
        Some(delay)
    }

    fn connected(&self) {
        let mut backoff = self.backoff.get();
        backoff.reset();
        self.backoff.set(backoff);
    }

    fn close_intentionally(&self) {
        self.intentionally_closed.set(true);
    }
}

impl ReconnectBackoff {
    pub fn new(base_delay: Duration, maximum_delay: Duration) -> Option<Self> {
        (base_delay > Duration::ZERO && maximum_delay >= base_delay).then_some(Self {
            base_delay,
            maximum_delay,
            retry_index: 0,
        })
    }

    /// Caps a caller generated random sample at min(base * 2^retry, maximum).
    pub fn delay(&self, jitter_sample: Duration) -> Duration {
        jitter_sample.min(self.retry_cap())
    }

    pub fn advance(&mut self) {
        self.retry_index = self.retry_index.saturating_add(1);
    }

    pub fn reset(&mut self) {
        self.retry_index = 0;
    }

    fn retry_cap(&self) -> Duration {
        let multiplier = 1_u32.checked_shl(self.retry_index).unwrap_or(u32::MAX);
        self.base_delay
            .checked_mul(multiplier)
            .unwrap_or(self.maximum_delay)
            .min(self.maximum_delay)
    }
}

type RpcResponse<Body> = Result<Response<Body>, RpcError>;

#[derive(Clone, Copy)]
enum StopReason {
    Cancelled,
    ConnectionClosed,
}

struct CallState {
    stop: Cell<Option<StopReason>>,
    active: Cell<bool>,
    outcome: Cell<CallOutcome>,
    waker: RefCell<Option<Waker>>,
}

impl CallState {
    fn stop(&self, reason: StopReason) {
        if self.active.get() && self.stop.get().is_none() {
            self.stop.set(Some(reason));
            self.outcome.set(match reason {
                StopReason::Cancelled => CallOutcome::Cancelled,
                StopReason::ConnectionClosed => CallOutcome::ConnectionClosed,
            });
            let waker = self.waker.borrow_mut().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    fn check(&self) -> Result<(), RpcError> {
        match self.stop.get() {
            None => Ok(()),
            Some(StopReason::Cancelled) => Err(RpcError::Cancelled),
            Some(StopReason::ConnectionClosed) => Err(RpcError::ConnectionClosed),
        }
    }
}

/// Cancels only its original request, including its returned response stream.
/// Retains the original local outcome after disposal, without retaining a socket.
/// A terminal handle cannot cancel a newer request or consume its admission slot.
#[derive(Clone)]
pub struct CallCancellation {
    state: Rc<CallState>,
}

impl CallCancellation {
    pub fn cancel(&self) {
        self.state.stop(StopReason::Cancelled);
    }

    pub fn outcome(&self) -> CallOutcome {
        self.state.outcome.get()
    }
}

struct ConnectionScope {
    closed: Rc<Cell<bool>>,
    calls: RefCell<Vec<Weak<CallState>>>,
}

impl ConnectionScope {
    fn admit(&self) -> Result<CallLease, RpcError> {
        if self.closed.get() {
            return Err(RpcError::ConnectionClosed);
        }
        let mut calls = self.calls.borrow_mut();
        calls.retain(|call| call.upgrade().is_some_and(|state| state.active.get()));
        if calls.len() >= CONCURRENT_STREAMS as usize {
            return Err(RpcError::Capacity);
        }
        let state = Rc::new(CallState {
            stop: Cell::new(None),
            active: Cell::new(true),
            outcome: Cell::new(CallOutcome::Waiting),
            waker: RefCell::new(None),
        });
        calls.push(Rc::downgrade(&state));
        Ok(CallLease { state })
    }

    fn close(&self) {
        self.closed.set(true);
        // Release the registry borrow before waking caller-owned futures.
        let calls = self
            .calls
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for call in calls {
            call.stop(StopReason::ConnectionClosed);
        }
    }
}

struct CallLease {
    state: Rc<CallState>,
}

impl CallLease {
    fn cancellation(&self) -> CallCancellation {
        CallCancellation {
            state: self.state.clone(),
        }
    }

    fn finish(&self, outcome: CallOutcome) {
        self.state.outcome.set(outcome);
        self.state.active.set(false);
        drop(self.state.waker.borrow_mut().take());
    }

    async fn wait<F: Future>(&self, future: F) -> Result<F::Output, RpcError> {
        let mut future = pin!(future);
        poll_fn(|context| {
            if let Err(error) = self.state.check() {
                return Poll::Ready(Err(error));
            }
            *self.state.waker.borrow_mut() = Some(context.waker().clone());
            match future.as_mut().poll(context) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(value) => {
                    drop(self.state.waker.borrow_mut().take());
                    Poll::Ready(self.state.check().map(|()| value))
                }
            }
        })
        .await
    }
}

impl Drop for CallLease {
    fn drop(&mut self) {
        if self.state.outcome.get() == CallOutcome::Waiting {
            self.state.outcome.set(CallOutcome::WaitDisposed);
        }
        self.state.active.set(false);
        drop(self.state.waker.borrow_mut().take());
    }
}

/// One generated-client connection generation. It owns no identity or game state.
/// Clone the generated client per request to multiplex without a lock across await.
/// Dropping/replacing this owner invalidates its pending calls and old streams.
/// Requests and response bodies remain caller-owned, local futures; no work is spawned.
/// The supplied generated client must use the approved message limits/interceptors.
pub struct RpcConnection<Client> {
    client: Client,
    scope: ConnectionScope,
    reconnect: ReconnectOwner,
    #[cfg(target_arch = "wasm32")]
    browser: Option<df_rpc_bridge::BrowserConnection>,
}

impl<Client: Clone> RpcConnection<Client> {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            scope: ConnectionScope {
                closed: Rc::new(Cell::new(false)),
                calls: RefCell::new(Vec::new()),
            },
            reconnect: ReconnectOwner::new(),
            #[cfg(target_arch = "wasm32")]
            browser: None,
        }
    }

    pub fn generation(&self) -> ConnectionGeneration {
        ConnectionGeneration {
            closed: self.scope.closed.clone(),
        }
    }

    /// Owns the existing browser bridge driver and uses its actual channel signature.
    /// The constructor supplies an actual generated client, with approved codec limits.
    #[cfg(target_arch = "wasm32")]
    pub async fn connect_browser(
        url: &str,
        construct: impl FnOnce(df_rpc_bridge::BrowserChannel) -> Client,
    ) -> std::io::Result<Self> {
        let (browser, channel) = df_rpc_bridge::BrowserConnection::connect(url).await?;
        Ok(Self::from_browser(construct(channel), browser))
    }

    /// Transfers the actual bridge owner after an existing generated-client qualifier.
    #[cfg(target_arch = "wasm32")]
    pub fn from_browser(client: Client, browser: df_rpc_bridge::BrowserConnection) -> Self {
        let mut connection = Self::new(client);
        connection.browser = Some(browser);
        connection
    }

    /// Read-only measurements from the existing bridge owner, when supplied.
    #[cfg(target_arch = "wasm32")]
    pub fn browser_snapshot(&self) -> std::io::Result<Option<df_rpc_bridge::ConnectionSnapshot>> {
        self.browser
            .as_ref()
            .map(|browser| browser.snapshot())
            .transpose()
    }

    /// Starts an actual generated unary or client-streaming call once, without retry.
    /// Request metadata, deadlines and input half-close belong to the generated call.
    /// Response metadata/extensions and Status details are preserved without parsing.
    pub fn unary<Body, Call>(
        &self,
        call: Call,
    ) -> Result<
        (
            CallCancellation,
            impl Future<Output = RpcResponse<Body>> + use<Client, Body, Call>,
        ),
        RpcError,
    >
    where
        Body: prost::Message,
        Call: AsyncFnOnce(&mut Client) -> Result<Response<Body>, Status>,
    {
        let lease = self.scope.admit()?;
        let cancellation = lease.cancellation();
        let mut client = self.client.clone();
        let response = async move {
            match lease.wait(call(&mut client)).await? {
                Ok(response) => {
                    lease.finish(CallOutcome::ResponseReceived);
                    Ok(response)
                }
                Err(status) => {
                    lease.finish(CallOutcome::GrpcStatus(status.code()));
                    Err(RpcError::Status(status))
                }
            }
        };
        Ok((cancellation, response))
    }

    /// Starts an actual generated server-streaming or bidirectional call once.
    /// The response stream retains its admission slot until closure or disposal.
    pub fn streaming<Message, Call>(
        &self,
        call: Call,
    ) -> Result<
        (
            CallCancellation,
            impl Future<Output = RpcResponse<RpcStream<Message>>> + use<Client, Message, Call>,
        ),
        RpcError,
    >
    where
        Call: AsyncFnOnce(&mut Client) -> Result<Response<Streaming<Message>>, Status>,
    {
        let lease = self.scope.admit()?;
        let cancellation = lease.cancellation();
        let mut client = self.client.clone();
        let response = async move {
            let response = match lease.wait(call(&mut client)).await? {
                Ok(response) => response,
                Err(status) => {
                    lease.finish(CallOutcome::GrpcStatus(status.code()));
                    return Err(RpcError::Status(status));
                }
            };
            Ok(response.map(|stream| RpcStream {
                stream: Some(stream),
                lease: Some(lease),
                drained: false,
            }))
        };
        Ok((cancellation, response))
    }

    pub fn close(&self) {
        self.reconnect.close_intentionally();
        self.scope.close();
        #[cfg(target_arch = "wasm32")]
        if let Some(browser) = &self.browser {
            browser.close();
        }
    }

    /// Returns the full-jitter delay for the next owned reconnect attempt.
    /// The owner supplies its uniform random sample and calls `connected` after success.
    pub fn schedule_reconnect(&self, jitter_sample: Duration) -> Option<Duration> {
        self.reconnect.schedule_after_disconnect(jitter_sample)
    }

    /// Resets retry growth after this connection owner establishes a successful connection.
    pub fn connected(&self) {
        self.reconnect.connected();
    }
}

impl<Client> Drop for RpcConnection<Client> {
    fn drop(&mut self) {
        self.scope.close();
        #[cfg(target_arch = "wasm32")]
        if let Some(browser) = &self.browser {
            browser.close();
        }
    }
}

/// Preserves generated message, EOF, error status and final trailers as separate facts.
/// Callers drain messages to EOF, then retrieve trailers. Drop cancels local receipt.
/// This wrapper never buffers, reconnects, replays input or acknowledges server work.
pub struct RpcStream<Message> {
    stream: Option<Streaming<Message>>,
    lease: Option<CallLease>,
    drained: bool,
}

impl<Message> RpcStream<Message> {
    pub async fn message(&mut self) -> Result<Option<Message>, RpcError> {
        let lease = self.lease.as_ref().ok_or(RpcError::StreamClosed)?;
        let stream = self.stream.as_mut().ok_or(RpcError::StreamClosed)?;
        let result = lease.wait(stream.message()).await;
        match result {
            Ok(Ok(message)) => {
                self.drained = message.is_none();
                Ok(message)
            }
            Ok(Err(status)) => {
                lease.finish(CallOutcome::GrpcStatus(status.code()));
                self.dispose();
                Err(RpcError::Status(status))
            }
            Err(error) => {
                self.dispose();
                Err(error)
            }
        }
    }

    /// Refuses premature draining so terminal access cannot silently lose messages.
    pub async fn trailers(&mut self) -> Result<Option<MetadataMap>, RpcError> {
        let lease = self.lease.as_ref().ok_or(RpcError::StreamClosed)?;
        if let Err(error) = lease.state.check() {
            self.dispose();
            return Err(error);
        }
        if !self.drained {
            return Err(RpcError::StreamNotDrained);
        }
        let stream = self.stream.as_mut().ok_or(RpcError::StreamClosed)?;
        let result = lease.wait(stream.trailers()).await;
        match &result {
            Ok(Ok(_)) => lease.finish(CallOutcome::StreamFinished),
            Ok(Err(status)) => lease.finish(CallOutcome::GrpcStatus(status.code())),
            Err(_) => {}
        }
        self.dispose();
        result?.map_err(RpcError::Status)
    }

    /// Releases only this local response/upload scope. Does not close other calls.
    pub fn cancel(&mut self) {
        if let Some(lease) = &self.lease {
            lease.state.stop(StopReason::Cancelled);
        }
        self.dispose();
    }

    fn dispose(&mut self) {
        drop(self.stream.take());
        drop(self.lease.take());
    }
}

#[cfg(test)]
mod reconnect_tests {
    use super::{ReconnectBackoff, RpcConnection};
    use std::time::Duration;

    #[test]
    fn retry_delay_uses_full_jitter_window_and_caps_exponential_growth() {
        let mut backoff = ReconnectBackoff::new(Duration::from_millis(250), Duration::from_secs(8))
            .expect("valid backoff bounds");
        assert_eq!(
            backoff.delay(Duration::from_millis(200)),
            Duration::from_millis(200)
        );
        assert_eq!(
            backoff.delay(Duration::from_secs(1)),
            Duration::from_millis(250)
        );
        backoff.advance();
        assert_eq!(
            backoff.delay(Duration::from_secs(1)),
            Duration::from_millis(500)
        );
        for _ in 0..8 {
            backoff.advance();
        }
        assert_eq!(
            backoff.delay(Duration::from_secs(20)),
            Duration::from_secs(8)
        );
        backoff.reset();
        assert_eq!(
            backoff.delay(Duration::from_secs(20)),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn retry_backoff_rejects_invalid_bounds() {
        assert!(ReconnectBackoff::new(Duration::ZERO, Duration::from_secs(1)).is_none());
        assert!(ReconnectBackoff::new(Duration::from_secs(2), Duration::from_secs(1)).is_none());
    }

    #[test]
    fn disconnect_retries_reset_after_success_and_stop_after_intentional_close() {
        let connection = RpcConnection::new(());
        assert_eq!(
            connection.schedule_reconnect(Duration::from_secs(1)),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            connection.schedule_reconnect(Duration::from_secs(1)),
            Some(Duration::from_millis(500))
        );
        connection.connected();
        assert_eq!(
            connection.schedule_reconnect(Duration::from_secs(1)),
            Some(Duration::from_millis(250))
        );
        connection.close();
        assert_eq!(connection.schedule_reconnect(Duration::ZERO), None);
    }

    #[test]
    fn replacing_a_connection_fences_the_old_client_scope() {
        let old_connection = RpcConnection::new(());
        let old_generation = old_connection.generation();
        let new_connection = RpcConnection::new(());
        let new_generation = new_connection.generation();

        old_connection.close();

        assert!(!old_generation.is_active());
        assert!(new_generation.is_active());
        assert!(!old_generation.same_scope(&new_generation));
    }
}
