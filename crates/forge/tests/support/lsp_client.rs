use async_lsp::{
    LanguageServer, MainLoop, ServerSocket,
    lsp_types::{
        PublishDiagnosticsParams, Url,
        notification::{self, Notification},
    },
    router::Router,
};
use futures::{
    future::{Either, select},
    pin_mut,
};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct LspClient {
    child: Option<Child>,
    pub(crate) runtime: tokio::runtime::Runtime,
    pub(crate) server: ServerSocket,
    main_loop: Option<JoinHandle<async_lsp::Result<()>>>,
    notifications: Receiver<String>,
    diagnostics: Receiver<PublishDiagnosticsParams>,
}

impl LspClient {
    pub fn spawn(project: &Path, path: &Path, args: &[&str]) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();

        let mut command = Command::new(env!("CARGO_BIN_EXE_forge"));
        command
            .current_dir(project)
            .env_remove("FOUNDRY_PROFILE")
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();

        let (notification_sender, notifications) = mpsc::channel();
        let (diagnostic_sender, diagnostics) = mpsc::channel();
        let (main_loop, server) = MainLoop::new_client(move |_| {
            let mut router = Router::new(());
            let log_sender = notification_sender.clone();
            router.notification::<notification::LogMessage>(move |_, _| {
                let _ = log_sender.send(notification::LogMessage::METHOD.to_owned());
                std::ops::ControlFlow::Continue(())
            });
            router.notification::<notification::PublishDiagnostics>(move |_, params| {
                let _ = diagnostic_sender.send(params);
                std::ops::ControlFlow::Continue(())
            });
            router.unhandled_notification(move |_, notification| {
                let _ = notification_sender.send(notification.method);
                std::ops::ControlFlow::Continue(())
            });
            router
        });

        let main_loop = thread::spawn(move || {
            let runtime =
                tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap();
            runtime.block_on(async move {
                let stdout = tokio::process::ChildStdout::from_std(stdout).unwrap();
                let stdin = tokio::process::ChildStdin::from_std(stdin).unwrap();
                main_loop.run_buffered(stdout.compat(), stdin.compat_write()).await
            })
        });

        Self {
            child: Some(child),
            runtime,
            server,
            main_loop: Some(main_loop),
            notifications,
            diagnostics,
        }
    }

    pub fn wait_for_log_message(&self) {
        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        let mut observed = Vec::new();
        loop {
            let timeout = deadline.saturating_duration_since(std::time::Instant::now());
            match self.notifications.recv_timeout(timeout) {
                Ok(method) if method == notification::LogMessage::METHOD => return,
                Ok(method) => observed.push(method),
                Err(RecvTimeoutError::Timeout) => {
                    panic!("timed out waiting for LSP log message; observed: {observed:?}")
                }
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("LSP client stopped before receiving a log message")
                }
            }
        }
    }

    pub fn wait_for_diagnostics(
        &self,
        uri: &Url,
        matches: impl Fn(&PublishDiagnosticsParams) -> bool,
    ) -> PublishDiagnosticsParams {
        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        let mut observed = Vec::new();
        loop {
            let timeout = deadline.saturating_duration_since(std::time::Instant::now());
            match self.diagnostics.recv_timeout(timeout) {
                Ok(params) if params.uri == *uri && matches(&params) => return params,
                Ok(params) => observed.push(params),
                Err(RecvTimeoutError::Timeout) => {
                    panic!(
                        "timed out waiting for LSP diagnostics for {uri}; observed: {observed:?}"
                    )
                }
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("LSP client stopped before receiving diagnostics for {uri}")
                }
            }
        }
    }

    pub fn shutdown(mut self) {
        let future = self.server.shutdown(());
        request(&self.runtime, future);
        self.server.exit(()).unwrap();

        // The server closes stdout after exit; let EOF finish the transport.
        let main_loop = self.main_loop.take().unwrap();
        let result = main_loop.join().unwrap();
        assert!(
            matches!(result, Err(async_lsp::Error::Eof)),
            "LSP client transport failed: {result:?}"
        );

        let mut child = self.child.take().unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "forge lsp exited unsuccessfully: {status}");
    }
}

pub(crate) fn request<T>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = async_lsp::Result<T>>,
) -> T {
    runtime.block_on(async move {
        let timeout = tokio::time::sleep(REQUEST_TIMEOUT);
        pin_mut!(timeout);
        pin_mut!(future);
        match select(future, timeout).await {
            Either::Left((result, _)) => result.unwrap_or_else(|error| {
                panic!("LSP request failed: {error}");
            }),
            Either::Right(((), _)) => panic!("timed out waiting for LSP response"),
        }
    })
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(main_loop) = self.main_loop.take() {
            let _ = main_loop.join();
        }
    }
}
