#![cfg(not(target_arch = "wasm32"))]

use bytes::Bytes;
use futures::future::{join_all, poll_fn};
use h2::{RecvStream, SendStream, client, server};
use std::task::Poll;
use tokio::{
    io::duplex,
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};

const HELD_PER_STREAM: usize = df_rpc_bridge::RECEIVE_BYTES / 4;

async fn send_bytes(send: &mut SendStream<Bytes>, mut remaining: usize) -> Result<(), h2::Error> {
    while remaining != 0 {
        let requested = remaining.min(df_rpc_bridge::FRAME_BYTES);
        send.reserve_capacity(requested);
        let capacity = poll_fn(|context| send.poll_capacity(context))
            .await
            .ok_or_else(|| h2::Error::from(h2::Reason::CANCEL))??;
        let length = requested.min(capacity);
        if length != 0 {
            send.send_data(Bytes::from(vec![7; length]), false)?;
            remaining -= length;
        }
    }
    Ok(())
}

async fn retain_window(body: &mut RecvStream) -> Vec<Bytes> {
    let mut retained = Vec::new();
    let mut received = 0;
    while received < HELD_PER_STREAM {
        let bytes = timeout(Duration::from_secs(5), body.data())
            .await
            .expect("h2 DATA did not arrive")
            .expect("h2 stream ended before its held window")
            .expect("h2 DATA failed");
        assert!(!bytes.is_empty(), "h2 returned empty nonterminal DATA");
        received += bytes.len();
        retained.push(bytes);
    }
    assert_eq!(received, HELD_PER_STREAM);
    retained
}

#[tokio::test]
async fn real_h2_aggregate_held_data_only_progresses_after_released_bytes() {
    let (client_io, server_io) = duplex(df_rpc_bridge::RECEIVE_BYTES * 2);
    let (ready_tx, mut ready_rx) = mpsc::channel(4);
    let (continue_first_tx, continue_first_rx) = oneshot::channel();
    let (first_stalled_tx, first_stalled_rx) = oneshot::channel();

    let (server_stop_tx, server_stop_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        let mut connection = server::handshake(server_io)
            .await
            .expect("h2 server handshake failed");
        let mut continue_first_rx = Some(continue_first_rx);
        let mut first_stalled_tx = Some(first_stalled_tx);
        let mut server_stop_rx = server_stop_rx;
        let mut producers = Vec::with_capacity(4);
        let mut stream_index = 0;
        loop {
            let accepted = tokio::select! {
                biased;
                _ = &mut server_stop_rx => break,
                accepted = connection.accept() => accepted,
            };
            let Some(result) = accepted else {
                break;
            };
            let (_, mut response) = result.expect("h2 server request failed");
            let mut send = response
                .send_response(http::Response::new(()), false)
                .expect("h2 response failed");
            let index = stream_index;
            stream_index += 1;
            let ready = ready_tx.clone();
            let (continue_first, first_stalled) = if index == 0 {
                (
                    Some(
                        continue_first_rx
                            .take()
                            .expect("first stream receiver missing"),
                    ),
                    Some(
                        first_stalled_tx
                            .take()
                            .expect("first stream stall sender missing"),
                    ),
                )
            } else {
                (None, None)
            };
            producers.push(tokio::spawn(async move {
                send_bytes(&mut send, HELD_PER_STREAM)
                    .await
                    .expect("initial h2 response DATA failed");
                ready
                    .send(())
                    .await
                    .expect("test receiver closed before initial DATA");

                if index == 0 {
                    continue_first
                        .expect("first stream continuation receiver missing")
                        .await
                        .expect("test did not release the first stream");
                    send.reserve_capacity(1);
                    let mut stalled =
                        Some(first_stalled.expect("first stream stall sender missing"));
                    let capacity = poll_fn(|context| match send.poll_capacity(context) {
                        Poll::Pending => {
                            if let Some(sender) = stalled.take() {
                                sender
                                    .send(())
                                    .expect("test receiver dropped before stall observation");
                            }
                            Poll::Pending
                        }
                        ready => ready,
                    })
                    .await
                    .expect("h2 stream closed before released credit")
                    .expect("h2 send capacity failed");
                    assert!(
                        capacity > 0,
                        "released credit did not restore send capacity"
                    );
                    send.send_data(Bytes::from_static(b"x"), false)
                        .expect("h2 DATA after release failed");
                }
                send.send_data(Bytes::new(), true)
                    .expect("h2 terminal DATA failed");
            }));
        }
        assert_eq!(stream_index, 4, "server did not accept all four h2 streams");
        for producer in producers {
            producer.await.expect("h2 response producer task failed");
        }
    });

    let (mut request, client_connection) = client::Builder::new()
        // A connection-sized test stream isolates aggregate exhaustion; production uses 256 KiB.
        .initial_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
        .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
        .handshake::<_, Bytes>(client_io)
        .await
        .expect("h2 client handshake failed");
    let client_driver = tokio::spawn(client_connection);

    let mut response_futures = Vec::with_capacity(4);
    for _ in 0..4 {
        let (response, _upload) = request
            .send_request(http::Request::new(()), true)
            .expect("h2 request failed");
        response_futures.push(response);
    }
    let responses = timeout(Duration::from_secs(5), join_all(response_futures))
        .await
        .expect("h2 responses did not arrive");
    let mut bodies: Vec<RecvStream> = responses
        .into_iter()
        .map(|response| response.expect("h2 response failed").into_body())
        .collect();

    for _ in 0..4 {
        timeout(Duration::from_secs(5), ready_rx.recv())
            .await
            .expect("server did not send the held windows")
            .expect("server producer stopped before the held windows");
    }
    let mut retained = Vec::with_capacity(4);
    for body in &mut bodies {
        retained.push(retain_window(body).await);
    }
    assert!(
        bodies
            .iter_mut()
            .all(|body| body.flow_control().used_capacity() == HELD_PER_STREAM),
        "polling DATA must not release capacity while the bytes remain held"
    );
    assert_eq!(
        bodies
            .iter_mut()
            .map(|body| body.flow_control().used_capacity())
            .sum::<usize>(),
        df_rpc_bridge::RECEIVE_BYTES,
        "four held streams should exhaust their shared connection receive window"
    );

    continue_first_tx
        .send(())
        .expect("first stream producer stopped");
    timeout(Duration::from_secs(5), first_stalled_rx)
        .await
        .expect("producer did not observe exhausted aggregate h2 capacity")
        .expect("producer stopped before observing the held-window stall");

    let consumed = retained[0].remove(0);
    bodies[0]
        .flow_control()
        .release_capacity(consumed.len())
        .expect("h2 rejected release for consumed DATA");
    drop(consumed);

    let resumed = timeout(Duration::from_secs(5), bodies[0].data())
        .await
        .expect("h2 DATA did not resume after capacity release")
        .expect("first h2 stream ended before resumed DATA")
        .expect("resumed h2 DATA failed");
    assert_eq!(resumed.as_ref(), b"x");
    bodies[0]
        .flow_control()
        .release_capacity(resumed.len())
        .expect("h2 rejected release for resumed DATA");
    drop(resumed);

    for (body, held) in bodies.iter_mut().zip(retained.iter_mut()) {
        for bytes in held.drain(..) {
            body.flow_control()
                .release_capacity(bytes.len())
                .expect("h2 rejected release for consumed held DATA");
            drop(bytes);
        }
    }
    for body in &mut bodies {
        if let Some(frame) = timeout(Duration::from_secs(5), body.data())
            .await
            .expect("h2 terminal frame did not arrive")
        {
            let bytes = frame.expect("h2 terminal DATA failed");
            assert!(
                bytes.is_empty(),
                "completed h2 body returned unexpected payload DATA"
            );
            assert!(
                body.is_end_stream(),
                "empty terminal DATA did not end h2 body"
            );
        }
        assert!(body.is_end_stream(), "h2 body did not reach end of stream");
        assert!(
            timeout(Duration::from_secs(5), body.data())
                .await
                .expect("h2 body did not complete after terminal frame")
                .is_none(),
            "h2 body returned DATA after end of stream"
        );
        assert_eq!(body.flow_control().used_capacity(), 0);
    }
    server_stop_tx
        .send(())
        .expect("server stopped before its owned producers were drained");
    client_driver.abort();
    match client_driver.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("h2 client driver failed during teardown: {error}"),
        Err(error) if error.is_cancelled() => {}
        Err(error) => panic!("h2 client driver join failed: {error}"),
    }
    timeout(Duration::from_secs(5), server_task)
        .await
        .expect("h2 server did not stop after its producer tasks drained")
        .expect("h2 server owner task failed");
}
