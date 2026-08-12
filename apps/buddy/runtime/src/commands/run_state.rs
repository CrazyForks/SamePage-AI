use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

use crate::{
    error::{BuddyError, BuddyResult},
    storage::{BuddyFinishedRun, BuddyRun, BuddyRunEvent},
};

struct BuddyRunWorker {
    cancellation: BuddyRunCancellationToken,
    conversation_id: Option<String>,
    join_handle: Option<JoinHandle<()>>,
}

#[derive(Clone, Default)]
pub struct BuddyRunCancellationRegistry {
    state: Arc<Mutex<BuddyRunRegistryState>>,
}

#[derive(Default)]
struct BuddyRunRegistryState {
    reserved_conversations: HashSet<String>,
    shutting_down: bool,
    workers: HashMap<String, BuddyRunWorker>,
}

pub struct BuddyConversationReservation {
    conversation_id: String,
    release_on_drop: bool,
    registry: BuddyRunCancellationRegistry,
}

impl Drop for BuddyConversationReservation {
    fn drop(&mut self) {
        if self.release_on_drop {
            self.registry
                .lock_state()
                .reserved_conversations
                .remove(&self.conversation_id);
        }
    }
}

impl BuddyConversationReservation {
    pub(super) fn register_run(mut self, run_id: &str) -> BuddyResult<BuddyRunCancellationToken> {
        let token = self
            .registry
            .register_reserved_run(run_id, self.conversation_id.clone())?;
        self.release_on_drop = false;
        Ok(token)
    }
}

impl BuddyRunCancellationRegistry {
    pub(crate) fn reserve_conversation(
        &self,
        conversation_id: &str,
    ) -> BuddyResult<BuddyConversationReservation> {
        let mut state = self.lock_state();
        if state.shutting_down {
            return Err(runtime_shutting_down_error());
        }
        if state.reserved_conversations.contains(conversation_id)
            || state
                .workers
                .values()
                .any(|worker| worker.conversation_id.as_deref() == Some(conversation_id))
        {
            return Err(conversation_busy_error(conversation_id));
        }

        state
            .reserved_conversations
            .insert(conversation_id.to_owned());
        Ok(BuddyConversationReservation {
            conversation_id: conversation_id.to_owned(),
            release_on_drop: true,
            registry: self.clone(),
        })
    }

    fn register_reserved_run(
        &self,
        run_id: &str,
        conversation_id: String,
    ) -> BuddyResult<BuddyRunCancellationToken> {
        let token = BuddyRunCancellationToken::new();
        let mut state = self.lock_state();
        state.reserved_conversations.remove(&conversation_id);
        if state.shutting_down {
            return Err(runtime_shutting_down_error());
        }
        state.workers.insert(
            run_id.to_owned(),
            BuddyRunWorker {
                cancellation: token.clone(),
                conversation_id: Some(conversation_id),
                join_handle: None,
            },
        );
        Ok(token)
    }

    #[cfg(test)]
    pub(super) fn register(
        &self,
        run_id: &str,
        conversation_id: Option<String>,
    ) -> BuddyRunCancellationToken {
        let token = BuddyRunCancellationToken::new();
        self.lock_state().workers.insert(
            run_id.to_owned(),
            BuddyRunWorker {
                cancellation: token.clone(),
                conversation_id,
                join_handle: None,
            },
        );

        token
    }

    pub(super) fn attach_worker(&self, run_id: &str, join_handle: JoinHandle<()>) {
        let mut join_handle = Some(join_handle);
        if let Some(worker) = self.lock_state().workers.get_mut(run_id) {
            worker.join_handle = join_handle.take();
        }

        if let Some(join_handle) = join_handle {
            let _ = join_handle.join();
        }
    }

    #[cfg(test)]
    pub(crate) fn ensure_conversation_idle(&self, conversation_id: &str) -> BuddyResult<()> {
        let state = self.lock_state();
        if state.reserved_conversations.contains(conversation_id)
            || state
                .workers
                .values()
                .any(|worker| worker.conversation_id.as_deref() == Some(conversation_id))
        {
            return Err(conversation_busy_error(conversation_id));
        }

        Ok(())
    }

    pub(super) fn cancel(&self, run_id: &str) -> bool {
        let Some(token) = self.token(run_id) else {
            return false;
        };

        token.request_cancel()
    }

    pub(super) fn remove(&self, run_id: &str) {
        self.lock_state().workers.remove(run_id);
    }

    pub(crate) fn shutdown(&self) {
        let workers = {
            let mut state = self.lock_state();
            state.shutting_down = true;
            state.reserved_conversations.clear();
            std::mem::take(&mut state.workers)
        };

        for worker in workers.values() {
            worker.cancellation.request_cancel();
        }
        for worker in workers.into_values() {
            if let Some(join_handle) = worker.join_handle {
                let _ = join_handle.join();
            }
        }
    }

    fn token(&self, run_id: &str) -> Option<BuddyRunCancellationToken> {
        self.lock_state()
            .workers
            .get(run_id)
            .map(|worker| worker.cancellation.clone())
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, BuddyRunRegistryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn conversation_busy_error(conversation_id: &str) -> BuddyError {
    BuddyError::Validation(format!(
        "conversation {conversation_id} already has an active run"
    ))
}

fn runtime_shutting_down_error() -> BuddyError {
    BuddyError::Runtime("runtime is shutting down".to_owned())
}

const RUN_STATE_RUNNING: u8 = 0;
const RUN_STATE_CANCEL_REQUESTED: u8 = 1;
const RUN_STATE_TERMINAL_CLAIMED: u8 = 2;

#[derive(Clone)]
pub(super) struct BuddyRunCancellationToken {
    runtime_cancellation: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
}

impl BuddyRunCancellationToken {
    fn new() -> Self {
        Self {
            runtime_cancellation: Arc::new(AtomicBool::new(false)),
            state: Arc::new(AtomicU8::new(RUN_STATE_RUNNING)),
        }
    }

    fn request_cancel(&self) -> bool {
        if self
            .state
            .compare_exchange(
                RUN_STATE_RUNNING,
                RUN_STATE_CANCEL_REQUESTED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return false;
        }

        self.runtime_cancellation.store(true, Ordering::SeqCst);
        true
    }

    pub(super) fn try_claim_terminal(&self) -> bool {
        self.state
            .compare_exchange(
                RUN_STATE_RUNNING,
                RUN_STATE_TERMINAL_CLAIMED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    pub(super) fn runtime_cancellation(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.runtime_cancellation)
    }

    fn is_cancel_requested(&self) -> bool {
        self.state.load(Ordering::SeqCst) == RUN_STATE_CANCEL_REQUESTED
    }
}

pub(super) fn is_buddy_run_cancelled(cancellation: Option<&BuddyRunCancellationToken>) -> bool {
    cancellation.is_some_and(BuddyRunCancellationToken::is_cancel_requested)
}

pub(super) fn claim_buddy_run_terminal(cancellation: Option<&BuddyRunCancellationToken>) -> bool {
    cancellation.is_none_or(BuddyRunCancellationToken::try_claim_terminal)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyRunStateChangedEvent {
    pub run_id: String,
    pub session_id: Option<String>,
    pub event_id: Option<i64>,
    pub event_type: Option<String>,
    pub status: Option<String>,
}

type BuddyRunStateEventSink = Arc<dyn Fn(BuddyRunStateChangedEvent) + Send + Sync>;

#[derive(Clone)]
pub(crate) struct BuddyRunStateEventPublisher {
    sink: Option<BuddyRunStateEventSink>,
}

impl BuddyRunStateEventPublisher {
    pub(crate) fn new(sink: impl Fn(BuddyRunStateChangedEvent) + Send + Sync + 'static) -> Self {
        Self {
            sink: Some(Arc::new(sink)),
        }
    }

    pub(super) fn disabled() -> Self {
        Self { sink: None }
    }

    pub(super) fn emit_event(&self, event: &BuddyRunEvent, session_id: Option<&str>) {
        self.emit(create_buddy_run_state_changed_event_from_event(
            event, session_id, None,
        ));
    }

    pub(super) fn emit_events(&self, events: &[BuddyRunEvent], session_id: Option<&str>) {
        for event in events {
            self.emit_event(event, session_id);
        }
    }

    pub(super) fn emit_run(&self, run: &BuddyRun) {
        self.emit(create_buddy_run_state_changed_event_from_run(run));
    }

    pub(super) fn emit_finished_run(&self, finished_run: &BuddyFinishedRun) {
        self.emit(create_buddy_run_state_changed_event_from_event(
            &finished_run.event,
            finished_run.run.session_id.as_deref(),
            Some(&finished_run.run.status),
        ));
        self.emit_run(&finished_run.run);
    }

    fn emit(&self, payload: BuddyRunStateChangedEvent) {
        if let Some(sink) = &self.sink {
            sink(payload);
        }
    }
}

fn create_buddy_run_state_changed_event_from_event(
    event: &BuddyRunEvent,
    session_id: Option<&str>,
    status: Option<&str>,
) -> BuddyRunStateChangedEvent {
    BuddyRunStateChangedEvent {
        run_id: event.run_id.clone(),
        session_id: session_id.map(str::to_owned),
        event_id: Some(event.id),
        event_type: Some(event.event_type.clone()),
        status: status.map(str::to_owned),
    }
}

fn create_buddy_run_state_changed_event_from_run(run: &BuddyRun) -> BuddyRunStateChangedEvent {
    BuddyRunStateChangedEvent {
        run_id: run.id.clone(),
        session_id: run.session_id.clone(),
        event_id: None,
        event_type: None,
        status: Some(run.status.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Barrier,
        },
        thread,
    };

    use super::BuddyRunCancellationRegistry;

    #[test]
    fn conversation_reservation_is_atomic() {
        let registry = BuddyRunCancellationRegistry::default();
        let start = Arc::new(Barrier::new(3));
        let finish = Arc::new(Barrier::new(3));
        let (result_sender, result_receiver) = mpsc::channel();
        let workers = (0..2)
            .map(|_| {
                let registry = registry.clone();
                let start = Arc::clone(&start);
                let finish = Arc::clone(&finish);
                let result_sender = result_sender.clone();
                thread::spawn(move || {
                    start.wait();
                    let reservation = registry.reserve_conversation("conversation-1");
                    result_sender
                        .send(reservation.is_ok())
                        .expect("send reservation result");
                    finish.wait();
                    drop(reservation);
                })
            })
            .collect::<Vec<_>>();

        start.wait();
        let accepted = [
            result_receiver.recv().expect("first result"),
            result_receiver.recv().expect("second result"),
        ];

        assert_eq!(accepted.into_iter().filter(|accepted| *accepted).count(), 1);

        finish.wait();
        for worker in workers {
            worker.join().expect("join reservation worker");
        }
    }

    #[test]
    fn conversation_reservation_transfers_to_active_run() {
        let registry = BuddyRunCancellationRegistry::default();
        let reservation = registry
            .reserve_conversation("conversation-1")
            .expect("reserve conversation");

        reservation.register_run("run-1").expect("register run");

        let error = registry
            .reserve_conversation("conversation-1")
            .err()
            .expect("active run must keep the conversation reserved");
        assert!(error.to_string().contains("already has an active run"));

        registry.remove("run-1");
        registry
            .reserve_conversation("conversation-1")
            .expect("completed run must release the conversation");
    }

    #[test]
    fn shutdown_rejects_new_conversation_reservations() {
        let registry = BuddyRunCancellationRegistry::default();

        registry.shutdown();

        let error = registry
            .reserve_conversation("conversation-1")
            .err()
            .expect("shutdown registry must reject new work");
        assert!(error.to_string().contains("shutting down"));
    }

    #[test]
    fn shutdown_rejects_reserved_run_registration() {
        let registry = BuddyRunCancellationRegistry::default();
        let reservation = registry
            .reserve_conversation("conversation-1")
            .expect("reserve conversation");

        registry.shutdown();

        let error = reservation
            .register_run("run-1")
            .err()
            .expect("shutdown registry must reject reserved work");
        assert!(error.to_string().contains("shutting down"));
    }

    #[test]
    fn run_cancellation_registry_cancels_only_the_registered_run() {
        let registry = BuddyRunCancellationRegistry::default();
        let first = registry.register("run-1", Some("conversation-1".to_owned()));
        let second = registry.register("run-2", Some("conversation-2".to_owned()));

        assert!(registry.cancel("run-1"));

        assert!(first.is_cancel_requested());
        assert!(!second.is_cancel_requested());
        registry.remove("run-1");
        assert!(!registry.cancel("run-1"));
    }

    #[test]
    fn active_conversation_cannot_start_a_second_run() {
        let registry = BuddyRunCancellationRegistry::default();
        registry.register("run-1", Some("conversation-1".to_owned()));

        let error = registry
            .ensure_conversation_idle("conversation-1")
            .expect_err("active conversation must be rejected");

        assert!(error.to_string().contains("already has an active run"));
    }

    #[test]
    fn shutdown_cancels_and_joins_registered_workers() {
        let registry = BuddyRunCancellationRegistry::default();
        let cancellation = registry.register("run-1", Some("conversation-1".to_owned()));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let worker = thread::spawn(move || {
            while !cancellation.is_cancel_requested() {
                thread::yield_now();
            }
            worker_finished.store(true, Ordering::SeqCst);
        });
        registry.attach_worker("run-1", worker);

        registry.shutdown();

        assert!(finished.load(Ordering::SeqCst));
        assert!(!registry.cancel("run-1"));
    }

    #[test]
    fn cancellation_requested_before_terminal_claim_wins() {
        let registry = BuddyRunCancellationRegistry::default();
        let cancellation = registry.register("run-1", Some("conversation-1".to_owned()));

        assert!(registry.cancel("run-1"));
        assert!(!cancellation.try_claim_terminal());
    }

    #[test]
    fn terminal_claim_before_cancellation_makes_cancel_too_late() {
        let registry = BuddyRunCancellationRegistry::default();
        let cancellation = registry.register("run-1", Some("conversation-1".to_owned()));

        assert!(cancellation.try_claim_terminal());
        assert!(!registry.cancel("run-1"));
    }
}
