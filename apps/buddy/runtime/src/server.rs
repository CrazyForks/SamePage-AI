use std::{
    io::{BufRead, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
};

use serde_json::{json, Value};

use crate::protocol::{
    dispatch_request, parse_request, RpcOutput, RpcRequest, RUNTIME_PROTOCOL_VERSION,
};

const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const REQUEST_QUEUE_CAPACITY: usize = 64;
const REQUEST_WORKER_COUNT: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("runtime startup failed: {0}")]
    Startup(String),

    #[error("runtime transport failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("runtime response serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub type RuntimeNotificationSink = Arc<dyn Fn(Value) + Send + Sync>;

pub trait RpcRequestHandler: Send + Sync + 'static {
    fn start(&self, _notifications: RuntimeNotificationSink) -> Result<(), String> {
        Ok(())
    }

    fn dispatch(&self, request: RpcRequest, notifications: RuntimeNotificationSink) -> RpcOutput;

    fn shutdown(&self) {}
}

struct RequestWorkerPool {
    sender: Option<SyncSender<RpcRequest>>,
    shutting_down: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl RequestWorkerPool {
    fn try_send(&self, request: RpcRequest) -> Result<(), TrySendError<RpcRequest>> {
        let Some(sender) = self.sender.as_ref() else {
            return Err(TrySendError::Disconnected(request));
        };
        sender.try_send(request)
    }

    fn shutdown(mut self) {
        self.shutting_down.store(true, Ordering::Release);
        self.sender.take();
        for worker in self.workers {
            let _ = worker.join();
        }
    }
}

pub struct BasicRuntimeHandler;

impl RpcRequestHandler for BasicRuntimeHandler {
    fn dispatch(&self, request: RpcRequest, _notifications: RuntimeNotificationSink) -> RpcOutput {
        dispatch_request(request)
    }
}

pub fn serve<R, W, H>(mut reader: R, writer: W, handler: H) -> Result<(), ServerError>
where
    R: BufRead,
    W: Write + Send + 'static,
    H: RpcRequestHandler,
{
    let writer = Arc::new(Mutex::new(writer));
    let notifications = create_notification_sink(Arc::clone(&writer));
    let handler = Arc::new(handler);
    if let Err(error) = handler.start(Arc::clone(&notifications)) {
        handler.shutdown();
        return Err(ServerError::Startup(error));
    }
    if let Err(error) = write_shared_message(
        &writer,
        &json!({
            "jsonrpc": "2.0",
            "method": "runtime.ready",
            "params": { "protocolVersion": RUNTIME_PROTOCOL_VERSION },
        }),
    ) {
        handler.shutdown();
        return Err(error);
    }
    let request_queue = match spawn_request_workers(
        Arc::clone(&handler),
        Arc::clone(&notifications),
        Arc::clone(&writer),
    ) {
        Ok(request_queue) => request_queue,
        Err(error) => {
            handler.shutdown();
            return Err(error);
        }
    };

    let result = (|| {
        loop {
            let line = match read_protocol_line(&mut reader)? {
                ProtocolLine::EndOfInput => break,
                ProtocolLine::TooLarge => {
                    write_shared_message(&writer, &invalid_request_response())?;
                    continue;
                }
                ProtocolLine::Data(line) => line,
            };

            let line = match std::str::from_utf8(&line) {
                Ok(line) => line,
                Err(_) => {
                    write_shared_message(&writer, &parse_error_response())?;
                    continue;
                }
            };
            let request = match parse_request(line) {
                Ok(request) => request,
                Err(error) => {
                    write_shared_message(&writer, &error.response_value())?;
                    continue;
                }
            };
            if request.is_control_request() {
                let output = handler.dispatch(request, Arc::clone(&notifications));
                let should_shutdown = output.should_shutdown();
                write_shared_message(&writer, &output.into_response_value())?;
                if should_shutdown {
                    break;
                }
                continue;
            }

            if let Err(error) = request_queue.try_send(request) {
                let request = match error {
                    TrySendError::Full(request) | TrySendError::Disconnected(request) => request,
                };
                write_shared_message(
                    &writer,
                    &RpcOutput::error(request.id(), -32001, "Runtime request queue is busy")
                        .into_response_value(),
                )?;
            }
        }

        Ok(())
    })();
    handler.shutdown();
    request_queue.shutdown();
    result
}

fn spawn_request_workers<H, W>(
    handler: Arc<H>,
    notifications: RuntimeNotificationSink,
    writer: Arc<Mutex<W>>,
) -> Result<RequestWorkerPool, ServerError>
where
    H: RpcRequestHandler,
    W: Write + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(REQUEST_QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut workers: Vec<thread::JoinHandle<()>> = Vec::with_capacity(REQUEST_WORKER_COUNT);

    for worker_index in 0..REQUEST_WORKER_COUNT {
        let handler = Arc::clone(&handler);
        let notifications = Arc::clone(&notifications);
        let writer = Arc::clone(&writer);
        let receiver = Arc::clone(&receiver);
        let worker_shutting_down = Arc::clone(&shutting_down);
        let worker = match thread::Builder::new()
            .name(format!("lexora-rpc-worker-{worker_index}"))
            .spawn(move || {
                run_request_worker(
                    receiver,
                    handler,
                    notifications,
                    writer,
                    worker_shutting_down,
                )
            }) {
            Ok(worker) => worker,
            Err(error) => {
                shutting_down.store(true, Ordering::Release);
                drop(sender);
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(ServerError::Io(error));
            }
        };
        workers.push(worker);
    }

    Ok(RequestWorkerPool {
        sender: Some(sender),
        shutting_down,
        workers,
    })
}

fn run_request_worker<H, W>(
    receiver: Arc<Mutex<Receiver<RpcRequest>>>,
    handler: Arc<H>,
    notifications: RuntimeNotificationSink,
    writer: Arc<Mutex<W>>,
    shutting_down: Arc<AtomicBool>,
) where
    H: RpcRequestHandler,
    W: Write + Send + 'static,
{
    loop {
        let request = {
            let Ok(receiver) = receiver.lock() else {
                return;
            };
            let Ok(request) = receiver.recv() else {
                return;
            };
            request
        };
        if shutting_down.load(Ordering::Acquire) {
            return;
        }
        let output = handler.dispatch(request, Arc::clone(&notifications));
        if let Err(error) = write_shared_message(&writer, &output.into_response_value()) {
            eprintln!("runtime RPC response failed: {error}");
            return;
        }
    }
}

fn create_notification_sink<W>(writer: Arc<Mutex<W>>) -> RuntimeNotificationSink
where
    W: Write + Send + 'static,
{
    Arc::new(move |value| {
        let _ = write_shared_message(&writer, &value);
    })
}

fn write_shared_message<W>(writer: &Arc<Mutex<W>>, value: &Value) -> Result<(), ServerError>
where
    W: Write,
{
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("runtime output lock was poisoned"))?;
    write_message(&mut *writer, value)
}

enum ProtocolLine {
    EndOfInput,
    Data(Vec<u8>),
    TooLarge,
}

fn read_protocol_line(reader: &mut impl BufRead) -> Result<ProtocolLine, std::io::Error> {
    let mut data = Vec::new();
    let mut too_large = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if data.is_empty() && !too_large {
                Ok(ProtocolLine::EndOfInput)
            } else if too_large {
                Ok(ProtocolLine::TooLarge)
            } else {
                Ok(ProtocolLine::Data(data))
            };
        }

        let newline_index = available.iter().position(|byte| *byte == b'\n');
        let payload_bytes = newline_index.unwrap_or(available.len());
        if !too_large {
            if data.len() + payload_bytes > MAX_MESSAGE_BYTES {
                too_large = true;
                data.clear();
            } else {
                data.extend_from_slice(&available[..payload_bytes]);
            }
        }

        let consumed = newline_index.map_or(available.len(), |index| index + 1);
        reader.consume(consumed);

        if newline_index.is_some() {
            if too_large {
                return Ok(ProtocolLine::TooLarge);
            }

            if data.last() == Some(&b'\r') {
                data.pop();
            }
            return Ok(ProtocolLine::Data(data));
        }
    }
}

fn write_message(writer: &mut impl Write, value: &Value) -> Result<(), ServerError> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn parse_error_response() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": { "code": -32700, "message": "Parse error" },
    })
}

fn invalid_request_response() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": { "code": -32600, "message": "Invalid Request" },
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Write},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Condvar, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };

    use serde_json::{json, Value};

    use crate::protocol::{dispatch_request, RpcOutput, RpcRequest};

    use super::{serve, BasicRuntimeHandler, RpcRequestHandler, RuntimeNotificationSink};

    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl SharedBuffer {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().expect("buffer lock").clone()
        }
    }

    impl Write for SharedBuffer {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("buffer lock poisoned"))?
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serves_ready_status_parse_error_and_shutdown_in_order() {
        let input = concat!(
            "{not-json}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"status\",\"method\":\"runtime.status\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"shutdown\",\"method\":\"runtime.shutdown\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"ignored\",\"method\":\"runtime.status\",\"params\":{}}\n",
        );
        let output = SharedBuffer::default();

        serve(
            Cursor::new(input.as_bytes()),
            output.clone(),
            BasicRuntimeHandler,
        )
        .expect("serve protocol");

        let messages = String::from_utf8(output.bytes())
            .expect("UTF-8 output")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("JSON output"))
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "jsonrpc": "2.0",
                    "method": "runtime.ready",
                    "params": { "protocolVersion": 2 },
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": "Parse error" },
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": "status",
                    "result": {
                        "name": "lexora-buddy-runtime",
                        "protocolVersion": 2,
                        "ready": true,
                    },
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": "shutdown",
                    "result": { "accepted": true },
                }),
            ]
        );
    }

    struct StartupNotificationHandler;

    impl RpcRequestHandler for StartupNotificationHandler {
        fn start(&self, notifications: RuntimeNotificationSink) -> Result<(), String> {
            notifications(json!({
                "jsonrpc": "2.0",
                "method": "pet.state",
                "params": { "status": "ready" },
            }));
            Ok(())
        }

        fn dispatch(
            &self,
            request: RpcRequest,
            _notifications: RuntimeNotificationSink,
        ) -> RpcOutput {
            dispatch_request(request)
        }
    }

    #[test]
    fn starts_owned_services_before_announcing_runtime_readiness() {
        let input =
            "{\"jsonrpc\":\"2.0\",\"id\":\"shutdown\",\"method\":\"runtime.shutdown\",\"params\":{}}\n";
        let output = SharedBuffer::default();

        serve(
            Cursor::new(input.as_bytes()),
            output.clone(),
            StartupNotificationHandler,
        )
        .expect("serve protocol");

        let messages = String::from_utf8(output.bytes())
            .expect("UTF-8 output")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("JSON output"))
            .collect::<Vec<_>>();

        assert_eq!(messages[0]["method"], json!("pet.state"));
        assert_eq!(messages[1]["method"], json!("runtime.ready"));
    }

    #[derive(Clone)]
    struct BlockingHandler {
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    #[derive(Clone)]
    struct GracefulShutdownHandler {
        release: Arc<(Mutex<bool>, Condvar)>,
        shutdown_called: Arc<AtomicBool>,
        worker_started: Arc<(Mutex<bool>, Condvar)>,
        worker_finished: Arc<AtomicBool>,
    }

    impl RpcRequestHandler for GracefulShutdownHandler {
        fn shutdown(&self) {
            self.shutdown_called.store(true, Ordering::SeqCst);
            let (released, release_signal) = &*self.release;
            *released.lock().expect("release lock") = true;
            release_signal.notify_all();
        }

        fn dispatch(
            &self,
            request: RpcRequest,
            _notifications: RuntimeNotificationSink,
        ) -> RpcOutput {
            let (id, method, _) = request.into_parts();
            match method.as_str() {
                "test.block" => {
                    let (started, start_signal) = &*self.worker_started;
                    *started.lock().expect("start lock") = true;
                    start_signal.notify_all();
                    let (released, release_signal) = &*self.release;
                    let mut released = released.lock().expect("release lock");
                    while !*released {
                        released = release_signal.wait(released).expect("release wait");
                    }
                    thread::sleep(Duration::from_millis(50));
                    self.worker_finished.store(true, Ordering::SeqCst);
                    RpcOutput::response(id, json!({ "completed": true }))
                }
                "runtime.shutdown" => {
                    let (started, start_signal) = &*self.worker_started;
                    let mut started = started.lock().expect("start lock");
                    while !*started {
                        started = start_signal.wait(started).expect("start wait");
                    }
                    RpcOutput::shutdown(id)
                }
                _ => RpcOutput::error(id, -32601, "Method not found"),
            }
        }
    }

    impl RpcRequestHandler for BlockingHandler {
        fn dispatch(
            &self,
            request: RpcRequest,
            _notifications: RuntimeNotificationSink,
        ) -> RpcOutput {
            let (id, method, _) = request.into_parts();
            match method.as_str() {
                "test.block" => {
                    let (released, release_signal) = &*self.release;
                    let mut released = released.lock().expect("release lock");
                    while !*released {
                        released = release_signal.wait(released).expect("release wait");
                    }
                    RpcOutput::response(id, json!({ "completed": true }))
                }
                "runtime.status" => RpcOutput::response(id, json!({ "ready": true })),
                "runtime.shutdown" => RpcOutput::shutdown(id),
                _ => RpcOutput::error(id, -32601, "Method not found"),
            }
        }
    }

    #[test]
    fn control_requests_remain_responsive_while_an_operation_is_blocked() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":\"blocked\",\"method\":\"test.block\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"status\",\"method\":\"runtime.status\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"shutdown\",\"method\":\"runtime.shutdown\",\"params\":{}}\n",
        );
        let output = SharedBuffer::default();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let server_output = output.clone();
        let server_release = Arc::clone(&release);
        let server = thread::spawn(move || {
            serve(
                Cursor::new(input.as_bytes()),
                server_output,
                BlockingHandler {
                    release: server_release,
                },
            )
            .expect("serve protocol");
        });

        let deadline = Instant::now() + Duration::from_millis(300);
        let status_responded_before_release = loop {
            let output = String::from_utf8(output.bytes()).expect("UTF-8 output");
            if output
                .lines()
                .any(|line| line.contains("\"id\":\"status\""))
            {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(5));
        };

        let (released, release_signal) = &*release;
        *released.lock().expect("release lock") = true;
        release_signal.notify_all();
        server.join().expect("server thread");

        assert!(status_responded_before_release);
    }

    #[test]
    fn shutdown_stops_handler_and_joins_request_workers_before_returning() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":\"blocked\",\"method\":\"test.block\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"shutdown\",\"method\":\"runtime.shutdown\",\"params\":{}}\n",
        );
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let shutdown_called = Arc::new(AtomicBool::new(false));
        let worker_started = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_finished = Arc::new(AtomicBool::new(false));

        serve(
            Cursor::new(input.as_bytes()),
            SharedBuffer::default(),
            GracefulShutdownHandler {
                release,
                shutdown_called: Arc::clone(&shutdown_called),
                worker_started,
                worker_finished: Arc::clone(&worker_finished),
            },
        )
        .expect("serve protocol");

        assert!(shutdown_called.load(Ordering::SeqCst));
        assert!(worker_finished.load(Ordering::SeqCst));
    }
}
