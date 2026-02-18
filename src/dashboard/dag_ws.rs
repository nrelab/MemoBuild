use crate::dashboard::metrics::{BuildEvent, BuildObserver};
use crate::remote_cache::RemoteCache;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct BroadcastObserver {
    tx: broadcast::Sender<BuildEvent>,
}

impl BroadcastObserver {
    pub fn new(tx: broadcast::Sender<BuildEvent>) -> Self {
        Self { tx }
    }
}

impl BuildObserver for BroadcastObserver {
    fn on_event(&self, event: BuildEvent) {
        let _ = self.tx.send(event);
    }
}

pub struct RemoteObserver<R: RemoteCache> {
    remote: Arc<R>,
}

impl<R: RemoteCache + 'static> RemoteObserver<R> {
    pub fn new(remote: Arc<R>) -> Self {
        Self { remote }
    }
}

impl<R: RemoteCache + 'static> BuildObserver for RemoteObserver<R> {
    fn on_event(&self, event: BuildEvent) {
        let remote = self.remote.clone();
        tokio::spawn(async move {
            let _ = remote.report_build_event(event).await;
        });
    }
}
