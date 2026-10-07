//! Caller-scoped cancellation and admission fencing for owned native work.
//! Callers release a job only after its process has stopped; this module owns
//! neither processes nor account authorization.
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};
use tokio::sync::Notify;

#[derive(Default)]
struct State {
    closed: bool,
    jobs: usize,
}

#[derive(Default)]
struct Scope {
    state: Mutex<State>,
    cancelled: Notify,
    idle: Notify,
}

static SCOPES: LazyLock<Mutex<HashMap<String, Arc<Scope>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn scope(key: &str) -> Arc<Scope> {
    SCOPES
        .lock()
        .expect("process scopes")
        .entry(key.to_owned())
        .or_default()
        .clone()
}

/// An admitted job. A retired scope rejects later jobs, including those whose
/// request authorization preceded retirement. Release after process quiescence.
pub struct Job(Arc<Scope>);

impl Job {
    pub fn begin(key: Option<&str>) -> Result<Self, &'static str> {
        let scope = key.map(scope).unwrap_or_default();
        {
            let mut state = scope.state.lock().expect("process scope state");
            if state.closed {
                return Err("Harness account is being removed");
            }
            state.jobs += 1;
        }
        Ok(Self(scope))
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.state.lock().expect("process scope state").closed
    }

    pub async fn cancelled(&self) {
        let notified = self.0.cancelled.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("process scope state");
        state.jobs -= 1;
        if state.jobs == 0 {
            self.0.idle.notify_waiters();
        }
    }
}

/// Permanently fence admissions in this process and request cancellation.
/// A drain acknowledges only that all callers released their owned jobs.
pub fn retire(key: &str) -> Drain {
    let scope = scope(key);
    scope.state.lock().expect("process scope state").closed = true;
    scope.cancelled.notify_waiters();
    Drain(scope)
}

pub struct Drain(Arc<Scope>);

impl Drain {
    pub async fn wait(&self) {
        loop {
            let notified = self.0.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.state.lock().expect("process scope state").jobs == 0 {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Poll};

    #[tokio::test]
    async fn retirement_fences_admission_waits_for_cleanup_and_preserves_other_scopes() {
        let removed = choruz_common::new_id();
        let retained = choruz_common::new_id();
        let first = Job::begin(Some(&removed)).unwrap();
        let other = Job::begin(Some(&retained)).unwrap();
        let drain = retire(&removed);
        first.cancelled().await;
        assert!(first.is_cancelled());
        assert!(!other.is_cancelled());
        assert!(Job::begin(Some(&removed)).is_err());
        let waiting = drain.wait();
        tokio::pin!(waiting);
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await
        );
        drop(first);
        waiting.await;
        assert!(Job::begin(Some(&retained)).is_ok());
    }
}
