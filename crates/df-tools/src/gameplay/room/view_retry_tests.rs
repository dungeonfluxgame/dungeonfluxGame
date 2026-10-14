use super::*;
use df_session::inbox::MAX_INBOX_ITEMS;

struct ViewReducer {
    inbox: super::super::InboxHandle<actor::Call>,
}

impl Reducer<actor::Call> for ViewReducer {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::PublicSceneAsset { reply } => {
                let _ = reply.send(Ok(String::new()));
            }
            actor::Call::View { reply, .. } => {
                let _ = reply.send(Ok((
                    super::super::LocalDemoRole::Display,
                    rpc::ViewMessage::default(),
                )));
                self.inbox.stop().expect("test inbox stops");
            }
            _ => panic!("unexpected actor call"),
        }
    }
}

#[test]
fn watch_view_retries_when_the_actor_queue_is_full() {
    let (inbox, actor_loop) = df_session::inbox::bounded_inbox::<actor::Call>();
    for _ in 0..MAX_INBOX_ITEMS {
        let (reply, _wait) = tokio::sync::oneshot::channel();
        assert!(
            inbox
                .try_submit(actor::Call::PublicSceneAsset { reply })
                .is_ok()
        );
    }

    let reducer_inbox = inbox.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut reducer = ViewReducer {
            inbox: reducer_inbox,
        };
        actor_loop.run(&mut reducer)
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("test runtime starts");
    let result = runtime.block_on(actor_view(
        &inbox,
        [7; 32],
        rpc::WatchViewRequest::default(),
    ));
    inbox.stop().expect("test inbox stops");
    worker
        .join()
        .expect("test actor joins")
        .expect("test actor drains");
    assert!(
        result.is_ok(),
        "watch view should be served after queue space opens: {result:?}"
    );
}
