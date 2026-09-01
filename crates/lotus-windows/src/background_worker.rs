use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerJoinPolicy {
    Always,
    WhenFinished,
}

pub(crate) struct BackgroundWorker {
    handle: Option<JoinHandle<()>>,
    stop: Option<Box<dyn FnOnce()>>,
    join_policy: WorkerJoinPolicy,
}

impl BackgroundWorker {
    pub(crate) fn new(
        handle: JoinHandle<()>,
        join_policy: WorkerJoinPolicy,
        stop: impl FnOnce() + 'static,
    ) -> Self {
        Self {
            handle: Some(handle),
            stop: Some(Box::new(stop)),
            join_policy,
        }
    }

    pub(crate) fn set_join_policy(&mut self, join_policy: WorkerJoinPolicy) {
        self.join_policy = join_policy;
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
        let Some(handle) = self.handle.take() else {
            return;
        };
        if self.join_policy == WorkerJoinPolicy::Always || handle.is_finished() {
            let _ = handle.join();
        }
    }
}

impl Drop for BackgroundWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
