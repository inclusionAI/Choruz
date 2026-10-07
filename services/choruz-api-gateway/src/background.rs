use std::{future::Future, sync::Arc};

pub(crate) struct WorkerGuard(tokio::task::AbortHandle);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn spawn(work: impl Future<Output = ()> + Send + 'static) -> Arc<WorkerGuard> {
    Arc::new(WorkerGuard(tokio::spawn(work).abort_handle()))
}
