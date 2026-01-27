// use signal_hook::{consts::SIGINT, iterator::Signals};
// ref: https://tokio.rs/tokio/topics/shutdown
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

// 全てのShutdownLockが消えるまで待つWait
#[derive(Debug, Clone)]
pub struct ShutdownLock {
    /// lock shutdown
    lock_sender: UnboundedSender<()>,
}

impl ShutdownLock {
    pub fn new(sender: UnboundedSender<()>) -> ShutdownLock {
        ShutdownLock {
            lock_sender: sender,
        }
    }
    pub fn unlock(self) {
        drop(self.lock_sender)
    }
    pub fn is_shutdown(&self) -> bool {
        self.lock_sender.is_closed()
    }
}

// 全てのShutdownLockが消えるまで待つWait
#[derive(Debug)]
pub struct ShutdownWait {
    /// The receive for waiting shutdown.
    wait_receiver: UnboundedReceiver<()>,
}

impl ShutdownWait {
    pub fn new(wait_receiver: UnboundedReceiver<()>) -> ShutdownWait {
        ShutdownWait { wait_receiver }
    }
    pub async fn wait(&mut self) {
        let r = self.wait_receiver.recv().await;
        tracing::debug!("shutdown wait released: {:?}", r);
    }
}

pub fn create_lock_and_wait() -> (ShutdownLock, ShutdownWait) {
    let (send, recv) = mpsc::unbounded_channel();
    (ShutdownLock::new(send), ShutdownWait::new(recv))
}

/// Wait for shutdown signal (SIGINT or SIGTERM)
///
/// On Unix: waits for SIGINT or SIGTERM
/// On Windows: waits for Ctrl+C only
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigint = signal(SignalKind::interrupt()).expect("failed to create SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to create SIGTERM handler");

        tokio::select! {
            _ = sigint.recv() => {
                tracing::info!("received SIGINT, initiating shutdown");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, initiating shutdown");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl_c");
        tracing::info!("received ctrl_c, initiating shutdown");
    }
}

/// Spawn a shutdown signal handler that broadcasts to a watch channel
pub fn spawn_shutdown_handler(
    shutdown_send: tokio::sync::watch::Sender<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_send.send(true);
    })
}
