//! Real TCP accepts initialization bytes but never replies; queued Quit must recover.
use kiaro_secret_chat::{
    grammers_mtsender::{InvocationError, SenderPool},
    grammers_session::{SessionData, storages::MemorySession},
};
use std::{io::ErrorKind, sync::Arc, time::Duration};
use tokio::{io::AsyncReadExt, net::TcpListener, sync::Notify};

pub fn run() {
    std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
            .block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let std::net::SocketAddr::V4(address) = listener.local_addr().unwrap() else { panic!() };
                let initialized = Arc::new(Notify::new());
                let accepted = initialized.clone();
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut header = [0; 4];
                    socket.read_exact(&mut header).await.unwrap();
                    accepted.notify_one();
                    std::future::pending::<()>().await;
                    drop(socket);
                });
                let mut data = SessionData { home_dc: 2, ..Default::default() };
                let option = data.dc_options.get_mut(&2).unwrap();
                option.ipv4 = address;
                option.auth_key = Some([7; 256]);
                let pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
                let handle = pool.handle.clone();
                let mut runner = tokio::spawn(pool.runner.run());
                let caller = handle.clone();
                let mut invocation = tokio::spawn(async move {
                    caller.raw_invoke_in_dc(2, vec![1, 2, 3, 4]).await
                });
                tokio::time::timeout(Duration::from_secs(5), initialized.notified()).await.unwrap();
                assert!(handle.quit(), "Quit must remain queued behind the stalled initialization");
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                let response = tokio::time::timeout(Duration::from_millis(100), &mut invocation).await;
                let bounded = matches!(response, Ok(Ok(Err(InvocationError::Io(ref error)))) if error.kind() == ErrorKind::TimedOut);
                if !bounded { invocation.abort(); }
                let stopped = tokio::time::timeout(Duration::from_millis(100), &mut runner).await;
                if stopped.is_err() { runner.abort(); }
                server.abort();
                tokio::time::resume();
                assert!(bounded, "stalled actual TCP initialization did not return a timeout error");
                assert!(stopped.is_ok(), "the actual runner did not process queued Quit after timeout");
            });
    }).join().unwrap();
}
