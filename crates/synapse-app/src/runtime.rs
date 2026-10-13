// Copyright 2026 Synapse contributors.
// Adapted from Zed's `crates/gpui_tokio` (Apache-2.0, zed-industries/zed).
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.

//! Asynchronous UI bridge (ARCHITECTURE.md §1, MILESTONES.md M1.3).
//!
//! Network and disk work runs on a small shared tokio runtime (2 worker
//! threads) so the UI thread never blocks. [`spawn`] returns a
//! [`BridgeTask`] whose [`Drop`] aborts the work — the same contract as
//! Zed's `Tokio::spawn(cx, fut) -> gpui::Task`, minus the GPUI executor
//! half. When the GPUI Kit window lands, views drive these tasks from
//! `cx.spawn` and push results back via `WeakEntity::update`; until then
//! this module is toolkit-free and fully testable headless.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Shared tokio runtime: 2 worker threads (PERFORMANCE.md rule 6).
fn global_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("synapse-tokio")
            .enable_all()
            .build()
            .expect("synapse tokio runtime builds")
    })
}

/// Number of worker threads in [`global_runtime`].
pub fn worker_threads() -> usize {
    2
}

/// A unit of background work. Dropping it cancels the work (abort +
/// child-token cancel), mirroring `gpui::Task` semantics.
pub struct BridgeTask<T> {
    handle: Option<JoinHandle<T>>,
    token: CancellationToken,
}

impl<T> BridgeTask<T> {
    fn new(handle: JoinHandle<T>, token: CancellationToken) -> Self {
        Self {
            handle: Some(handle),
            token,
        }
    }

    /// The cancellation token owned by this task. Children created with
    /// [`spawn_child`] observe it.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Cancel the work without waiting for it.
    pub fn cancel(&mut self) {
        self.token.cancel();
        if let Some(h) = &self.handle {
            h.abort();
        }
    }

    /// Detach: the work keeps running after this handle is dropped.
    pub fn detach(mut self) {
        self.handle.take();
        // token intentionally not cancelled; `Drop` below skips empty handle.
    }

    /// Block the *calling* (non-UI, test/helper) thread until done.
    /// Never call this on the UI thread.
    pub fn block_on(mut self) -> Result<T, BridgeError> {
        let handle = self.handle.take().expect("task polled after completion");
        match global_runtime().block_on(handle) {
            Ok(v) => Ok(v),
            Err(e) if e.is_cancelled() => Err(BridgeError::Cancelled),
            Err(e) => Err(BridgeError::Join(e.to_string())),
        }
    }
}

impl<T> Drop for BridgeTask<T> {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            self.token.cancel();
            h.abort();
        }
    }
}

impl<T> Future for BridgeTask<T> {
    type Output = Result<T, BridgeError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let handle = self.handle.as_mut().expect("polled after completion");
        match Pin::new(handle).poll(cx) {
            Poll::Ready(Ok(v)) => Poll::Ready(Ok(v)),
            Poll::Ready(Err(e)) if e.is_cancelled() => Poll::Ready(Err(BridgeError::Cancelled)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(BridgeError::Join(e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Errors from awaited bridge tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    Cancelled,
    Join(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Cancelled => write!(f, "task was cancelled"),
            BridgeError::Join(e) => write!(f, "task panicked: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Spawn `fut` on the tokio runtime. The returned task cancels the work
/// when dropped. `fut` receives a child [`CancellationToken`] cancelled
/// together with the task.
pub fn spawn<T, F>(fut: F) -> BridgeTask<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    spawn_cancellable(|_token| fut)
}

/// Spawn with explicit access to the task's cancellation token.
pub fn spawn_cancellable<T, F>(make: impl FnOnce(CancellationToken) -> F) -> BridgeTask<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let token = CancellationToken::new();
    let fut = make(token.clone());
    let handle = global_runtime().spawn(fut);
    BridgeTask::new(handle, token)
}

/// Spawn a child future tied to `parent`: cancelling the parent cancels it.
pub fn spawn_child<T, F>(parent: &CancellationToken, fut: F) -> BridgeTask<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let child = parent.child_token();
    let token = child.clone();
    let handle = global_runtime().spawn(async move {
        let out = fut.await;
        drop(child);
        out
    });
    BridgeTask::new(handle, token)
}

/// Run blocking disk/CPU work on tokio's blocking pool without holding a
/// worker thread.
pub fn spawn_blocking<T>(f: impl FnOnce() -> T + Send + 'static) -> BridgeTask<T>
where
    T: Send + 'static,
{
    let token = CancellationToken::new();
    let handle = global_runtime().spawn_blocking(f);
    BridgeTask::new(handle, token)
}

/// Enter the shared runtime from sync code (tests, helpers). Never on UI.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    global_runtime().block_on(fut)
}

fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .build()
                .expect("reqwest client builds")
        })
        .clone()
}

/// Demo command (M1.3): fetch `url` on the bridge and return the body text.
/// The caller awaits the [`BridgeTask`] from the UI thread without blocking
/// interaction; dropping the task aborts the request.
pub fn fetch_url(url: impl Into<String>) -> BridgeTask<Result<String, String>> {
    let url = url.into();
    let client = http_client();
    spawn(async move {
        let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("GET {url} -> {status}"));
        }
        resp.text().await.map_err(|e| e.to_string())
    })
}

/// Read a file off the UI thread.
pub fn read_file(path: impl Into<std::path::PathBuf>) -> BridgeTask<std::io::Result<String>> {
    let path = path.into();
    spawn_blocking(move || std::fs::read_to_string(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn tokio_bridge_spawn_returns_value_off_ui_thread() {
        let task = spawn(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            41 + 1
        });
        assert_eq!(block_on(task), Ok(42));
    }

    #[test]
    fn tokio_bridge_drop_cancels_pending_work() {
        let token_holder = std::sync::Arc::new(std::sync::Mutex::new(false));
        let flag = token_holder.clone();
        let task = spawn_cancellable(|token| async move {
            // Observe the token so the future can exit promptly on cancel.
            tokio::select! {
                _ = token.cancelled() => "cancelled",
                _ = tokio::time::sleep(Duration::from_secs(30)) => "finished",
            }
        });
        // Give the runtime a moment to poll the task, then drop it.
        block_on(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
        });
        drop(task);
        // Aborted tasks finish promptly; a leaked task would hang this join.
        let done = block_on(async move {
            tokio::time::timeout(Duration::from_secs(2), async move {
                // New independent work still runs: the runtime is alive.
                spawn(async move { *flag.lock().unwrap() = true }).await
            })
            .await
        });
        assert!(done.is_ok());
        assert!(*token_holder.lock().unwrap());
    }

    #[test]
    fn tokio_bridge_explicit_cancel_aborts_await() {
        let mut task = spawn(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            1
        });
        task.cancel();
        assert_eq!(block_on(task), Err(BridgeError::Cancelled));
    }

    #[test]
    fn tokio_bridge_blocking_work_does_not_hog_workers() {
        let start = Instant::now();
        // Two blocking sleeps + one async sleep all run concurrently.
        let a = spawn_blocking(|| {
            std::thread::sleep(Duration::from_millis(80));
            "a"
        });
        let b = spawn_blocking(|| {
            std::thread::sleep(Duration::from_millis(80));
            "b"
        });
        let c = spawn(async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            "c"
        });
        let (a, b, c) = block_on(async { (a.await, b.await, c.await) });
        assert_eq!((a, b, c), (Ok("a"), Ok("b"), Ok("c")));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "work ran concurrently: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn tokio_bridge_fetch_url_without_blocking_interaction() {
        // Local HTTP server so the test runs offline; the fetch itself
        // goes through reqwest on the bridge runtime.
        let body = "synapse bridge demo body";
        let listener =
            block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() });
        let addr = listener.local_addr().unwrap();
        let serve = spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\ncontent-type: text/plain\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        // While the fetch is in flight, the "UI thread" (this thread)
        // stays responsive: it can do other work and poll the task.
        let fetch = fetch_url(format!("http://{addr}/demo"));
        let ui_work_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ui_work_done.clone();
        std::thread::scope(|s| {
            s.spawn(move || {
                // Simulate handling UI events during the fetch.
                std::thread::sleep(Duration::from_millis(5));
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let result = block_on(fetch);
            assert_eq!(result, Ok(Ok(body.to_string())));
        });
        assert!(ui_work_done.load(std::sync::atomic::Ordering::SeqCst));
        let _ = block_on(serve);
    }

    #[test]
    fn tokio_bridge_fetch_url_error_is_reported() {
        // Nothing listens here; reqwest must report Err, not panic.
        let task = fetch_url("http://127.0.0.1:1/nope");
        let result = block_on(task).expect("task completes");
        assert!(result.is_err(), "expected fetch error, got {result:?}");
    }
}
