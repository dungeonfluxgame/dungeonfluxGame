//! The typed-dialogue deadline applies to both gRPC and the browser's local wait.
use std::{future::Future, time::Duration};

pub(crate) async fn submit<T, R, Send, Reply, Timer, Deadline>(
    mut request: tonic::Request<T>,
    timeout: Duration,
    send: Send,
    timer: Timer,
) -> Result<R, tonic::Status>
where
    Send: FnOnce(tonic::Request<T>) -> Reply,
    Reply: Future<Output = Result<R, tonic::Status>>,
    Timer: FnOnce(Duration) -> Deadline,
    Deadline: Future<Output = ()>,
{
    request.set_timeout(timeout);
    let reply = send(request);
    let deadline = timer(timeout);
    futures::pin_mut!(reply, deadline);
    match futures::future::select(reply, deadline).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(((), _)) => {
            Err(tonic::Status::deadline_exceeded("dialogue stalled"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{channel::oneshot, future::poll_fn, task::noop_waker_ref};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        task::{Context, Poll},
    };

    fn timer(clock: Arc<AtomicU64>, timeout: Duration) -> impl Future<Output = ()> {
        let deadline_ms = u64::try_from(timeout.as_millis()).unwrap();
        poll_fn(move |_| {
            if clock.load(Ordering::SeqCst) >= deadline_ms {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
    }

    #[test]
    fn advertised_codex_timeout_allows_a_reply_after_the_old_twenty_second_limit() {
        let clock = Arc::new(AtomicU64::new(0));
        let timer_clock = Arc::clone(&clock);
        let (sender, receiver) = oneshot::channel::<Result<&'static str, tonic::Status>>();
        let call = submit(
            tonic::Request::new("question"),
            Duration::from_secs(305),
            |request| {
                assert_eq!(request.metadata().get("grpc-timeout").unwrap(), "305000m");
                async move { receiver.await.unwrap() }
            },
            |timeout| timer(timer_clock, timeout),
        );
        futures::pin_mut!(call);
        let mut context = Context::from_waker(noop_waker_ref());
        assert!(call.as_mut().poll(&mut context).is_pending());
        clock.store(21_000, Ordering::SeqCst);
        assert!(call.as_mut().poll(&mut context).is_pending());
        sender.send(Ok("reply")).unwrap();
        assert!(matches!(
            call.as_mut().poll(&mut context),
            Poll::Ready(Ok("reply"))
        ));
    }

    #[test]
    fn default_timeout_bounds_a_stalled_reply_and_drops_the_request() {
        let clock = Arc::new(AtomicU64::new(0));
        let timer_clock = Arc::clone(&clock);
        let (sender, receiver) = oneshot::channel::<Result<(), tonic::Status>>();
        let call = submit(
            tonic::Request::new("question"),
            Duration::from_secs(25),
            |request| {
                assert_eq!(request.metadata().get("grpc-timeout").unwrap(), "25000000u");
                async move { receiver.await.unwrap() }
            },
            |timeout| timer(timer_clock, timeout),
        );
        futures::pin_mut!(call);
        let mut context = Context::from_waker(noop_waker_ref());
        assert!(call.as_mut().poll(&mut context).is_pending());
        clock.store(25_000, Ordering::SeqCst);
        let Poll::Ready(Err(error)) = call.as_mut().poll(&mut context) else {
            panic!("stalled dialogue did not reach its deadline");
        };
        assert_eq!(error.code(), tonic::Code::DeadlineExceeded);
        assert!(sender.is_canceled());
    }
}
