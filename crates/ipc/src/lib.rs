//! Transport to the Nolurix core.
//!
//! The core does not create the endpoint itself: it connects to ours through a
//! Unix socket or Windows named pipe, checking that the peer is its own parent
//! and that the parent's binary is nearby and named `nolurix`.
//!
//! Frames (little-endian):
//!   request: [u32 req_id][u16 method_len][method][u32 payload_len][payload]
//!   response: [u32 req_id][u8 status][u32 data_len][data]
//! status != 0 — the response body is an error message.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

#[cfg(target_os = "macos")]
mod service_macos;
#[cfg(target_os = "macos")]
pub use service_macos::unregister_helper;

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(unix)]
type IpcStream = UnixStream;
#[cfg(windows)]
type IpcStream = NamedPipeServer;

pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/libcore.rs"));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn request(peer: &mut UnixStream) -> u32 {
        let id = peer.read_u32_le().await.unwrap();
        let len = peer.read_u16_le().await.unwrap();
        let mut method = vec![0; len as usize];
        peer.read_exact(&mut method).await.unwrap();
        let len = peer.read_u32_le().await.unwrap();
        let mut payload = vec![0; len as usize];
        peer.read_exact(&mut payload).await.unwrap();
        id
    }

    #[tokio::test]
    async fn disconnect_fails_pending_and_future() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let client = CoreClient::attach(stream);
        let other = client.clone();
        let pending = tokio::spawn(async move { other.stop().await });
        request(&mut peer).await;
        drop(peer);
        assert!(tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(client.is_closed());
        assert!(client.stop().await.is_err());
    }

    #[tokio::test]
    async fn close_joins_owned_tasks_invalidates_clones_and_caches_failure() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::create(dir.path()).unwrap();
        let path = endpoint.path.clone();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let (client, tasks) = CoreClient::attach_owned(stream);
        let clone = client.clone();
        let mut core = Core {
            child: None,
            _endpoint: Some(endpoint),
            tasks,
            closed: false,
            close_error: None,
            client,
        };
        let error = core.close(Duration::from_millis(20)).await.unwrap_err();
        assert!(core.tasks.0.is_empty());
        assert!(!path.parent().unwrap().exists());
        assert!(clone.is_closed());
        assert!(clone.query_stats().await.is_err());
        assert_eq!(
            format!("{error:#}"),
            core.close(Duration::ZERO).await.unwrap_err().to_string()
        );
    }

    #[tokio::test]
    async fn owner_drop_closes_both_halves_with_clone() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::create(dir.path()).unwrap();
        let path = endpoint.path.clone();
        let listener = UnixListener::bind(&path).unwrap();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let core = Core {
            child: None,
            _endpoint: Some(endpoint),
            tasks: Tasks(Vec::new()),
            closed: false,
            close_error: None,
            client: CoreClient::attach(stream),
        };
        let clone = core.client.clone();
        drop(core);
        assert!(clone.is_closed());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read_u8())
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert!(!path.exists());
        drop(listener);
    }

    #[tokio::test]
    async fn oversize_response_closes_transport() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let client = CoreClient::attach(stream);
        let other = client.clone();
        let call = tokio::spawn(async move { other.stop().await });
        let id = request(&mut peer).await;
        peer.write_u32_le(id).await.unwrap();
        peer.write_u8(0).await.unwrap();
        peer.write_u32_le(MAX_FRAME + 1).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(1), call)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn cancellation_removes_pending_and_timeout_closes() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let client = CoreClient::attach(stream);
        let other = client.clone();
        let call = tokio::spawn(async move { other.stop().await });
        request(&mut peer).await;
        call.abort();
        let _ = call.await;
        assert!(client.transport.pending.lock().unwrap().is_empty());
        assert!(!client.is_closed());
        let result: Result<pb::EmptyResp> = client
            .call_with_timeout("Stop", pb::EmptyReq {}, Duration::from_millis(20))
            .await;
        assert!(result.is_err());
        assert!(client.is_closed());
        assert!(client.transport.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn external_timeout_removes_pending_and_allows_next_query() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let client = CoreClient::attach(stream);
        let other = client.clone();
        let timed = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(20), other.query_stats()).await
        });
        let canceled_id = request(&mut peer).await;
        assert!(timed.await.unwrap().is_err());
        assert!(client.transport.pending.lock().unwrap().is_empty());
        assert!(!client.is_closed());

        let other = client.clone();
        let next = tokio::spawn(async move { other.query_stats().await });
        let next_id = request(&mut peer).await;
        // A late response to the canceled call must not poison the next query.
        for id in [canceled_id, next_id] {
            peer.write_u32_le(id).await.unwrap();
            peer.write_u8(0).await.unwrap();
            peer.write_u32_le(0).await.unwrap();
        }
        assert!(tokio::time::timeout(Duration::from_secs(1), next)
            .await
            .unwrap()
            .unwrap()
            .is_ok());
        assert!(client.transport.pending.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn early_eof_and_already_closed_watch_release_both_tasks() {
        let (closed, _) = watch::channel(false);
        closed.send_replace(true);
        let mut late = closed.subscribe();
        tokio::time::timeout(Duration::from_secs(1), late.wait_for(|value| *value))
            .await
            .unwrap()
            .unwrap();
        for _ in 0..64 {
            let (stream, peer) = UnixStream::pair().unwrap();
            drop(peer);
            let client = CoreClient::attach(stream);
            let clone = client.clone();
            let weak = Arc::downgrade(&client.transport);
            tokio::time::timeout(Duration::from_secs(1), async {
                while !client.is_closed() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(clone.stop().await.is_err());
            drop(client);
            drop(clone);
            tokio::time::timeout(Duration::from_secs(1), async {
                while weak.upgrade().is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn admission_timeout_and_outgoing_validation() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let client = CoreClient::attach(stream);
        let result: Result<pb::EmptyResp> = client
            .call(
                "Start",
                pb::LoadConfigReq {
                    core_config: Some("x".repeat(MAX_FRAME as usize)),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err());
        assert!(!client.is_closed());
        let permits = client.transport.control.acquire_many(8).await.unwrap();
        let result: Result<pb::EmptyResp> = client
            .call_with_timeout("Stop", pb::EmptyReq {}, Duration::from_millis(10))
            .await;
        assert!(result.is_err());
        assert!(client.is_closed());
        drop(permits);
    }

    #[tokio::test]
    async fn peer_credentials_require_expected_pid() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert!(authenticate(&stream, Some(std::process::id()), false).unwrap());
        assert!(!authenticate(&stream, Some(std::process::id().wrapping_add(1)), false).unwrap());
        assert!(!authenticate(&stream, None, false).unwrap());
    }

    #[tokio::test]
    async fn long_runtime_parent_falls_back_with_private_isolated_cleanup() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        // Multibyte names: socket limits count bytes, not characters.
        let long = root.path().join("é".repeat(60));
        std::fs::create_dir(&long).unwrap();
        let foreign = long.join("s");
        std::fs::write(&foreign, b"do not remove").unwrap();
        let first = Endpoint::create(&long).unwrap();
        let second = Endpoint::create(&long).unwrap();
        assert!(!first.path.starts_with(&long));
        assert_ne!(first.path, second.path);
        let first_dir = first.path.parent().unwrap().to_owned();
        let second_dir = second.path.parent().unwrap().to_owned();
        assert_eq!(std::fs::canonicalize(&first_dir).unwrap(), first_dir);
        assert_eq!(
            std::fs::metadata(&first_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let listener = UnixListener::bind(&first.path).unwrap();
        let other_listener = UnixListener::bind(&second.path).unwrap();
        std::fs::set_permissions(&first.path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            std::fs::metadata(&first.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(listener);
        drop(first);
        assert!(!first_dir.exists());
        assert!(second_dir.exists());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"do not remove");
        drop(other_listener);
        drop(second);
        assert!(!second_dir.exists());
    }

    #[tokio::test]
    async fn endpoints_are_private_isolated_and_owned() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let first = Endpoint::create(dir.path()).unwrap();
        let second = Endpoint::create(dir.path()).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(
            std::fs::metadata(first.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let path = first.path.clone();
        let _listener = UnixListener::bind(&path).unwrap();
        drop(first);
        assert!(!path.exists());
        assert!(second.path.parent().unwrap().exists());
    }
}

/// Maximum frame size. The core is trusted, but the channel still must not be
/// able to make us allocate an arbitrary amount of memory from a malformed header.
const MAX_FRAME: u32 = 64 * 1024 * 1024;

type Pending = StdMutex<HashMap<u32, oneshot::Sender<Result<Vec<u8>>>>>;

struct Transport {
    closed: watch::Sender<bool>,
    pending: Pending,
    next_id: AtomicU32,
    work: Semaphore,
    control: Semaphore,
}

impl Transport {
    fn close(&self) {
        self.closed.send_replace(true);
        self.pending.lock().unwrap().clear();
    }
}

struct PendingGuard(Arc<Transport>, u32, Arc<AtomicBool>);
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.2.store(true, Ordering::SeqCst);
        self.0.pending.lock().unwrap().remove(&self.1);
    }
}

struct Call {
    id: u32,
    canceled: Arc<AtomicBool>,
    method: &'static str,
    payload: Vec<u8>,
    reply: oneshot::Sender<Result<Vec<u8>>>,
}

struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
/// Client for one live connection to the core. Can be cloned freely.
#[derive(Clone)]
pub struct CoreClient {
    tx: mpsc::Sender<Call>,
    transport: Arc<Transport>,
}

/// The running core process together with its channel.
pub struct Core {
    child: Option<Child>,
    _endpoint: Option<Endpoint>,
    tasks: Tasks,
    closed: bool,
    close_error: Option<String>,
    pub client: CoreClient,
}

struct Endpoint {
    path: PathBuf,
    #[cfg(unix)]
    _directory: tempfile::TempDir,
}

impl Endpoint {
    fn create(socket_dir: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // TempDir atomically creates a fresh private 0700 directory. Its guard
            // exists before any await, including helper startup and accept.
            std::fs::create_dir_all(socket_dir)?;
            let create = |parent: &Path| -> Result<tempfile::TempDir> {
                // Pass a canonical path to the helper: on macOS /tmp itself is
                // a symlink, and helper validation rejects symlink ancestors.
                let parent = std::fs::canonicalize(parent)?;
                Ok(tempfile::Builder::new()
                    .prefix("c-")
                    .permissions(std::fs::Permissions::from_mode(0o700))
                    .tempdir_in(parent)?)
            };
            let mut directory = create(socket_dir)?;
            // Match the target's sockaddr_un limit (including the terminating
            // NUL), measured in OS bytes rather than Unicode characters. A long
            // configured runtime root must not make IPC impossible. Never use a
            // shared socket name: the fallback still owns a fresh 0700 TempDir.
            if std::os::unix::net::SocketAddr::from_pathname(directory.path().join("s")).is_err() {
                directory = create(Path::new("/tmp"))?;
            }
            std::os::unix::net::SocketAddr::from_pathname(directory.path().join("s"))
                .context("IPC socket path exceeds the target Unix socket limit")?;
            Ok(Self {
                path: directory.path().join("s"),
                _directory: directory,
            })
        }
        #[cfg(windows)]
        {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let _ = socket_dir;
            Ok(Self {
                path: PathBuf::from(format!(
                    r"\\.\pipe\nolurix-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_nanos(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                )),
            })
        }
    }
}

impl Core {
    /// Creates the IPC endpoint, starts `core_bin`, and waits for it to connect.
    pub async fn spawn(core_bin: &Path, socket_dir: &Path, debug: bool) -> Result<Self> {
        Self::spawn_inner(core_bin, socket_dir, debug, false).await
    }

    /// Starts the core through the macOS root-helper. Other platforms retain
    /// their existing privilege mechanism and spawn the core normally.
    pub async fn spawn_privileged(core_bin: &Path, socket_dir: &Path, debug: bool) -> Result<Self> {
        #[cfg(target_os = "macos")]
        return Self::spawn_inner(core_bin, socket_dir, debug, true).await;

        #[cfg(not(target_os = "macos"))]
        Self::spawn(core_bin, socket_dir, debug).await
    }

    async fn spawn_inner(
        core_bin: &Path,
        socket_dir: &Path,
        debug: bool,
        privileged: bool,
    ) -> Result<Self> {
        let owned_endpoint = Endpoint::create(socket_dir)?;
        let endpoint = &owned_endpoint.path;

        #[cfg(unix)]
        let listener = {
            use std::os::unix::fs::PermissionsExt;
            let listener = UnixListener::bind(endpoint)
                .with_context(|| format!("bind {}", endpoint.display()))?;
            std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600))?;
            listener
        };
        #[cfg(windows)]
        let listener = ServerOptions::new()
            // Refuse to attach to an endpoint planted before us. The core also
            // verifies the server PID after connecting.
            .first_pipe_instance(true)
            .create(&endpoint)
            .with_context(|| format!("создание {}", endpoint.display()))?;

        let mut tasks = Tasks(Vec::new());
        let mut child;
        let expected_pid;
        if privileged {
            #[cfg(target_os = "macos")]
            {
                expected_pid = Some(service_macos::start_privileged_core(&endpoint, debug).await?);
                child = None;
            }
            #[cfg(not(target_os = "macos"))]
            unreachable!("privileged helper is only used on macOS");
        } else {
            let mut cmd = Command::new(core_bin);
            cmd.env("NOLURIX_CORE_SOCKET", &endpoint)
                .env("NOLURIX_CORE_DEBUG", if debug { "1" } else { "0" })
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut spawned = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    return Err(e).with_context(|| format!("запуск ядра {}", core_bin.display()));
                }
            };
            tasks.0.extend(pipe_core_logs(&mut spawned));
            expected_pid = spawned.id();
            child = Some(spawned);
        }

        // The core retries the connection ten times at 500 ms intervals; wait with some margin.
        #[cfg(unix)]
        let accept = async {
            loop {
                let (stream, _) = listener.accept().await?;
                if authenticate(&stream, expected_pid, privileged)? {
                    break Ok::<_, std::io::Error>(stream);
                }
            }
        };
        #[cfg(windows)]
        let accept = async {
            listener.connect().await?;
            Ok::<_, std::io::Error>(listener)
        };

        let accept = tokio::time::timeout(std::time::Duration::from_secs(15), accept);
        let stream = match accept.await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                if let Some(child) = child.as_mut() {
                    let _ = tokio::time::timeout(Duration::from_secs(5), child.kill()).await;
                }
                return Err(e).context("accept от ядра");
            }
            Err(_) => {
                if let Some(child) = child.as_mut() {
                    let _ = tokio::time::timeout(Duration::from_secs(5), child.kill()).await;
                }
                bail!(
                    "ядро не подключилось к {} за 15 с — проверьте, что бинарь GUI называется \
                     `nolurix` и лежит рядом с ядром",
                    endpoint.display()
                );
            }
        };

        let (client, mut transport_tasks) = CoreClient::attach_owned(stream);
        tasks.0.append(&mut transport_tasks.0);
        Ok(Self {
            child,
            _endpoint: Some(owned_endpoint),
            tasks,
            closed: false,
            close_error: None,
            client,
        })
    }

    /// Attempts a bounded graceful Stop, then closes all transport clones and
    /// reaps a directly owned child. An error means VPN restoration is unconfirmed.
    pub async fn close(&mut self, grace: Duration) -> Result<()> {
        if self.closed {
            return match &self.close_error {
                Some(e) => Err(anyhow!(e.clone())),
                None => Ok(()),
            };
        }
        let stop = tokio::time::timeout(grace, self.client.stop())
            .await
            .map_err(|_| anyhow!("таймаут Stop; восстановление VPN не подтверждено"))
            .and_then(|r| r)
            .context("Stop: восстановление VPN не подтверждено");
        self.client.transport.close();
        for task in &self.tasks.0 {
            task.abort();
        }
        for task in &mut self.tasks.0 {
            let _ = task.await;
        }
        self.tasks.0.clear();
        let reap = if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => Ok(()),
                Ok(None) => match child.start_kill() {
                    Ok(()) => tokio::time::timeout(Duration::from_secs(5), child.wait())
                        .await
                        .context("таймаут завершения ядра")
                        .and_then(|r| r.map(|_| ()).map_err(Into::into)),
                    Err(e) => Err(e.into()),
                },
                Err(e) => Err(e.into()),
            }
        } else {
            Ok(())
        };
        self._endpoint.take();
        self.closed = true;
        let result = stop.and(reap);
        self.close_error = result.as_ref().err().map(|e| format!("{e:#}"));
        result
    }

    pub async fn shutdown(mut self) {
        let _ = self.close(Duration::from_secs(5)).await;
    }

    /// Whether the core process has exited on its own.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.client.transport.close();
        for task in &self.tasks.0 {
            task.abort();
        }
    }
}

#[cfg(unix)]
fn authenticate(
    stream: &UnixStream,
    expected_pid: Option<u32>,
    privileged: bool,
) -> std::io::Result<bool> {
    let cred = stream.peer_cred()?;
    // No UID-only fallback, including privileged helper starts.
    let pid = peer_pid(stream)?;
    let expected_uid = if privileged {
        0
    } else {
        unsafe { libc::geteuid() }
    };
    Ok(expected_pid.is_some() && pid == expected_pid && cred.uid() == expected_uid)
}

#[cfg(unix)]
fn peer_pid(stream: &UnixStream) -> std::io::Result<Option<u32>> {
    #[cfg(not(target_os = "macos"))]
    let cred = stream.peer_cred()?;
    #[cfg(not(target_os = "macos"))]
    let pid = cred.pid().map(|pid| pid as u32);
    #[cfg(target_os = "macos")]
    let pid = {
        use std::os::fd::AsRawFd;
        let mut value: libc::pid_t = 0;
        let mut len = std::mem::size_of_val(&value) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                0,
                libc::LOCAL_PEERPID,
                (&mut value as *mut libc::pid_t).cast(),
                &mut len,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Some(value as u32)
    };
    Ok(pid)
}

fn pipe_core_logs(child: &mut Child) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::new();
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "core", "{line}");
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "core", "{line}");
            }
        }));
    }
    tasks
}

/// Request to start the configuration.
///
/// Constructing `pb::LoadConfigReq` manually is dangerous: the core dereferences
/// some fields without checking for nil, and omitting `need_extra_process` causes
/// it to panic after the configuration has already passed validation. All such
/// fields are always filled in here.
pub fn load_request(core_config: String) -> pb::LoadConfigReq {
    pb::LoadConfigReq {
        core_config: Some(core_config),
        need_extra_process: Some(false),
        need_xray: Some(false),
        xray_config: Some(String::new()),
        disable_stats: Some(false),
        tun_ipv4_cidr: Some(String::new()),
        ..Default::default()
    }
}

/// The same applies to the speed test request: the core dereferences its own set
/// of fields there.
pub fn speed_test_request(config: String) -> pb::SpeedTestRequest {
    pb::SpeedTestRequest {
        config: Some(config),
        test_download: Some(false),
        test_upload: Some(false),
        simple_download: Some(false),
        simple_download_addr: Some(String::new()),
        only_country: Some(false),
        country_concurrency: Some(4),
        timeout_ms: Some(10_000),
        need_xray: Some(false),
        xray_config: Some(String::new()),
        ..Default::default()
    }
}

impl CoreClient {
    #[cfg(test)]
    fn attach(stream: IpcStream) -> Self {
        let (client, mut tasks) = Self::attach_owned(stream);
        // Legacy transport-only tests allow tasks to finish on transport close.
        tasks.0.clear();
        client
    }

    fn attach_owned(stream: IpcStream) -> (Self, Tasks) {
        let mut tasks = Tasks(Vec::new());
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::channel::<Call>(12);
        let (closed, _) = watch::channel(false);
        let transport = Arc::new(Transport {
            closed,
            pending: StdMutex::default(),
            next_id: AtomicU32::new(1),
            work: Semaphore::new(4),
            control: Semaphore::new(8),
        });
        {
            let state = transport.clone();
            let mut closed = state.closed.subscribe();
            tasks.0.push(tokio::spawn(async move {
                let run = async {
                    while let Some(call) = rx.recv().await {
                        if call.reply.is_closed() {
                            continue;
                        }
                        let id = call.id;
                        {
                            let mut pending = state.pending.lock().unwrap();
                            if call.canceled.load(Ordering::SeqCst) {
                                continue;
                            }
                            pending.insert(id, call.reply);
                        }
                        // Write without another payload-sized frame allocation.
                        writer.write_all(&id.to_le_bytes()).await?;
                        writer
                            .write_all(&(call.method.len() as u16).to_le_bytes())
                            .await?;
                        writer.write_all(call.method.as_bytes()).await?;
                        writer
                            .write_all(&(call.payload.len() as u32).to_le_bytes())
                            .await?;
                        writer.write_all(&call.payload).await?;
                    }
                    Ok::<_, std::io::Error>(())
                };
                tokio::select! { biased; _ = closed.wait_for(|value| *value) => {}, _ = run => {} }
                state.close();
                // Dropping this half and the reader's half closes the OS handle.
            }));
        }
        {
            let state = transport.clone();
            let mut closed = state.closed.subscribe();
            tasks.0.push(tokio::spawn(async move {
                let run = async {
                    loop {
                        let mut head = [0u8; 9];
                        reader.read_exact(&mut head).await?;
                        let id = u32::from_le_bytes(head[0..4].try_into().unwrap());
                        let len = u32::from_le_bytes(head[5..9].try_into().unwrap());
                        if len > MAX_FRAME {
                            bail!("oversized core response");
                        }
                        let mut data = vec![0u8; len as usize];
                        reader.read_exact(&mut data).await?;
                        if let Some(reply) = state.pending.lock().unwrap().remove(&id) {
                            let _ = reply.send(if head[4] == 0 {
                                Ok(data)
                            } else {
                                Err(anyhow!(String::from_utf8_lossy(&data).into_owned()))
                            });
                        }
                    }
                    #[allow(unreachable_code)]
                    Ok::<_, anyhow::Error>(())
                };
                tokio::select! { biased; _ = closed.wait_for(|value| *value) => {}, _ = run => {} }
                state.close();
            }));
        }
        (Self { tx, transport }, tasks)
    }

    pub fn is_closed(&self) -> bool {
        *self.transport.closed.borrow()
    }

    async fn call<Req: Message, Resp: Message + Default>(
        &self,
        method: &'static str,
        req: Req,
    ) -> Result<Resp> {
        self.call_with_timeout(method, req, Duration::from_secs(15))
            .await
    }

    async fn call_with_timeout<Req: Message, Resp: Message + Default>(
        &self,
        method: &'static str,
        req: Req,
        budget: Duration,
    ) -> Result<Resp> {
        let deadline = tokio::time::Instant::now() + budget;
        if self.is_closed() {
            bail!("core connection closed");
        }
        if method.len() > u16::MAX as usize || req.encoded_len() > MAX_FRAME as usize {
            bail!("oversized core request");
        }
        let mut closed = self.transport.closed.subscribe();
        let work = async {
            // Separate control admission means StopTest/queries remain usable while
            // tests are outstanding. At most 12 encoded requests (4 work + 8
            // control), each <=64MiB, plus one response. Caller-owned protobufs
            // waiting for admission remain outside this transport memory bound.
            let semaphore = if matches!(
                method,
                "Test" | "SpeedTest" | "IPTest" | "Start" | "CheckConfig"
            ) {
                &self.transport.work
            } else {
                &self.transport.control
            };
            let _permit = semaphore.acquire().await?;
            if self.is_closed() {
                bail!("core connection closed");
            }
            let id = self.transport.next_id.fetch_add(1, Ordering::Relaxed);
            // Do not risk correlating a wrapped ID with a late response.
            if id == u32::MAX {
                self.transport.close();
                bail!("request ID exhausted");
            }
            let canceled = Arc::new(AtomicBool::new(false));
            let _guard = PendingGuard(self.transport.clone(), id, canceled.clone());
            let (reply, wait) = oneshot::channel();
            self.tx
                .send(Call {
                    id,
                    canceled,
                    method,
                    payload: req.encode_to_vec(),
                    reply,
                })
                .await
                .map_err(|_| anyhow!("core connection closed"))?;
            let data = wait
                .await
                .map_err(|_| anyhow!("core connection closed"))??;
            Resp::decode(data.as_slice()).with_context(|| format!("разбор ответа {method}"))
        };
        tokio::select! {
            biased;
            _ = closed.wait_for(|value| *value) => Err(anyhow!("core connection closed")),
            result = tokio::time::timeout_at(deadline, work) => match result {
                Ok(result) => result,
                Err(_) => {
                    // A write or operation may already have happened. Fail closed
                    // rather than continuing with uncertain core state.
                    self.transport.close();
                    Err(anyhow!("core RPC {method} timed out"))
                }
            }
        }
    }

    /// Some methods respond with `ErrorResp`, where the error is a regular field
    /// rather than a nonzero frame status. Convert it to `Err` so callers do not
    /// have to check two different error channels.
    async fn call_checked<Req: Message>(&self, method: &'static str, req: Req) -> Result<()> {
        let resp: pb::ErrorResp = self.call(method, req).await?;
        match resp.error() {
            "" => Ok(()),
            e => Err(anyhow!(e.to_string())),
        }
    }

    pub async fn start(&self, req: pb::LoadConfigReq) -> Result<()> {
        self.call_checked("Start", req).await
    }

    pub async fn stop(&self) -> Result<()> {
        self.call_checked("Stop", pb::EmptyReq {}).await
    }

    pub async fn check_config(&self, req: pb::LoadConfigReq) -> Result<()> {
        self.call_checked("CheckConfig", req).await
    }

    pub async fn test(&self, req: pb::TestReq) -> Result<pb::TestResp> {
        let budget = latency_budget(&req);
        self.call_with_timeout("Test", req, budget).await
    }

    pub async fn stop_test(&self) -> Result<()> {
        let _: pb::EmptyResp = self.call("StopTest", pb::EmptyReq {}).await?;
        Ok(())
    }

    pub async fn query_url_test(&self) -> Result<pb::QueryUrlTestResponse> {
        self.call("QueryURLTest", pb::EmptyReq {}).await
    }

    pub async fn ip_test(&self, req: pb::IpTestRequest) -> Result<pb::IpTestResp> {
        let seconds = 120
            + req.outbound_tags.len().max(1) as u64
                * 4
                * (req.test_timeout_ms().max(30_000) as u64 / 1000 + 1);
        self.call_with_timeout("IPTest", req, Duration::from_secs(seconds))
            .await
    }

    pub async fn query_ip_test(&self) -> Result<pb::QueryIpTestResponse> {
        self.call("QueryIPTest", pb::EmptyReq {}).await
    }

    pub async fn speed_test(&self, req: pb::SpeedTestRequest) -> Result<pb::SpeedTestResponse> {
        let budget = speed_budget(&req);
        self.call_with_timeout("SpeedTest", req, budget).await
    }

    pub async fn query_speed_test(&self) -> Result<pb::QuerySpeedTestResponse> {
        self.call("QuerySpeedTest", pb::EmptyReq {}).await
    }

    pub async fn query_country_test(&self) -> Result<pb::QueryCountryTestResponse> {
        self.call("QueryCountryTest", pb::EmptyReq {}).await
    }

    pub async fn query_stats(&self) -> Result<pb::QueryStatsResp> {
        self.call("QueryStats", pb::EmptyReq {}).await
    }

    pub async fn query_connections(&self) -> Result<pb::QueryConnectionsResp> {
        self.call("QueryConnections", pb::EmptyReq {}).await
    }

    pub async fn query_auto_selectors(&self) -> Result<pb::QueryAutoSelectorsResponse> {
        self.call("QueryAutoSelectors", pb::EmptyReq {}).await
    }

    pub async fn auto_selector_action(&self, req: pb::AutoSelectorActionRequest) -> Result<()> {
        self.call_checked("AutoSelectorAction", req).await
    }

    pub async fn is_privileged(&self) -> Result<bool> {
        let r: pb::IsPrivilegedResponse = self.call("IsPrivileged", pb::EmptyReq {}).await?;
        Ok(r.has_privilege())
    }

    pub async fn set_system_dns(&self, clear: bool) -> Result<()> {
        let _: pb::EmptyResp = self
            .call(
                "SetSystemDNS",
                pb::SetSystemDnsRequest { clear: Some(clear) },
            )
            .await?;
        Ok(())
    }

    pub async fn default_interface(&self) -> Result<(String, i32)> {
        let r: pb::GetDefaultInterfaceResponse =
            self.call("GetDefaultInterface", pb::EmptyReq {}).await?;
        Ok((r.name().to_string(), r.index()))
    }

    pub async fn gen_wg_keypair(&self) -> Result<(String, String)> {
        let r: pb::GenWgKeyPairResponse = self.call("GenWgKeyPair", pb::EmptyReq {}).await?;
        match r.error() {
            "" => Ok((r.private_key().to_string(), r.public_key().to_string())),
            e => Err(anyhow!(e.to_string())),
        }
    }
}

#[cfg(all(test, unix))]
mod safety_tests {
    use super::*;

    async fn request(peer: &mut UnixStream) -> u32 {
        let id = peer.read_u32_le().await.unwrap();
        let len = peer.read_u16_le().await.unwrap();
        let mut method = vec![0; len as usize];
        peer.read_exact(&mut method).await.unwrap();
        let len = peer.read_u32_le().await.unwrap();
        let mut payload = vec![0; len as usize];
        peer.read_exact(&mut payload).await.unwrap();
        id
    }

    async fn finish(client: &CoreClient, mut tasks: Tasks) {
        client.transport.close();
        for task in &mut tasks.0 {
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
        tasks.0.clear();
    }

    #[tokio::test]
    async fn pending_removed_on_timeout_drop_and_late_reply() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let (client, tasks) = CoreClient::attach_owned(stream);
        let c = client.clone();
        let call = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(50), c.query_stats()).await
        });
        let id = request(&mut peer).await;
        assert!(call.await.unwrap().is_err());
        assert_eq!(client.transport.pending.lock().unwrap().len(), 0);
        peer.write_all(&[id.to_le_bytes().as_slice(), &[0], &0u32.to_le_bytes()].concat())
            .await
            .unwrap();
        let c = client.clone();
        let call = tokio::spawn(async move { c.query_stats().await });
        request(&mut peer).await;
        call.abort();
        let _ = call.await;
        assert_eq!(client.transport.pending.lock().unwrap().len(), 0);
        // A late response must not poison the next request.
        let c = client.clone();
        let call = tokio::spawn(async move { c.query_stats().await });
        let id = request(&mut peer).await;
        peer.write_all(&[id.to_le_bytes().as_slice(), &[0], &0u32.to_le_bytes()].concat())
            .await
            .unwrap();
        call.await.unwrap().unwrap();
        finish(&client, tasks).await;
    }

    #[tokio::test]
    async fn close_invalidates_clones_and_preserves_failure() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let (client, tasks) = CoreClient::attach_owned(stream);
        let clone = client.clone();
        let mut core = Core {
            child: None,
            _endpoint: None,
            tasks,
            closed: false,
            close_error: None,
            client,
        };
        assert!(core.close(Duration::from_millis(20)).await.is_err());
        assert!(core.tasks.0.is_empty());
        assert!(clone.query_stats().await.is_err());
        assert_eq!(clone.transport.pending.lock().unwrap().len(), 0);
        assert!(core.close(Duration::from_secs(1)).await.is_err());
    }

    #[tokio::test]
    async fn writer_failure_wakes_all_and_joins() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let (client, tasks) = CoreClient::attach_owned(stream);
        let c = client.clone();
        let first = tokio::spawn(async move {
            c.call_with_timeout::<_, pb::EmptyResp>(
                "Test",
                pb::EmptyReq {},
                Duration::from_secs(30),
            )
            .await
        });
        request(&mut peer).await;
        // Stop reading but retain the peer write half: failure must originate
        // in the writer rather than relying on EOF from the response reader.
        let (peer_read, _peer_write) = peer.into_split();
        drop(peer_read);
        let c = client.clone();
        let second = tokio::spawn(async move {
            c.call_with_timeout::<_, pb::EmptyResp>(
                "Test",
                pb::LoadConfigReq {
                    core_config: Some("x".repeat(8 * 1024 * 1024)),
                    ..Default::default()
                },
                Duration::from_secs(1),
            )
            .await
        });
        assert!(tokio::time::timeout(Duration::from_secs(8), first)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(second.await.unwrap().is_err());
        assert_eq!(client.transport.pending.lock().unwrap().len(), 0);
        finish(&client, tasks).await;
    }

    #[test]
    fn endpoint_guards_are_independent_and_preserve_replacements() {
        let root = tempfile::tempdir().unwrap();
        let ga = Endpoint::create(root.path()).unwrap();
        let gb = Endpoint::create(root.path()).unwrap();
        let a = ga.path.clone();
        let b = gb.path.clone();
        let _a_listener = std::os::unix::net::UnixListener::bind(&a).unwrap();
        let _b_listener = std::os::unix::net::UnixListener::bind(&b).unwrap();
        // Foreign entries outside an owned private directory survive cleanup.
        let replacement = root.path().join("replacement");
        std::fs::write(&replacement, b"foreign").unwrap();
        drop(ga);
        assert!(!a.exists());
        assert!(b.exists());
        drop(gb);
        assert_eq!(std::fs::read(replacement).unwrap(), b"foreign");
    }

    #[test]
    fn concurrent_endpoint_names_unique() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..100)
                        .map(|_| Endpoint::create(Path::new("/tmp")).unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let names: std::collections::HashSet<_> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .map(|endpoint| endpoint.path.clone())
            .collect();
        assert_eq!(names.len(), 800);
    }

    #[test]
    fn long_latency_deadline_arithmetic() {
        let mut req = pb::TestReq {
            outbound_tags: vec![String::new(); 501],
            ..Default::default()
        };
        assert_eq!(latency_budget(&req), Duration::from_secs(11082));
        req.max_concurrency = Some(500);
        assert_eq!(latency_budget(&req), Duration::from_secs(11082));
        req.max_concurrency = Some(2);
        req.test_timeout_ms = Some(10_000);
        assert_eq!(latency_budget(&req), Duration::from_secs(11082));
        let speed = pb::SpeedTestRequest {
            outbound_tags: vec![String::new(); 10],
            test_download: Some(true),
            test_upload: Some(true),
            timeout_ms: Some(60_000),
            ..Default::default()
        };
        assert_eq!(speed_budget(&speed), Duration::from_secs(1940));
    }
}

fn latency_budget(req: &pb::TestReq) -> Duration {
    Duration::from_secs(
        60 + req.outbound_tags.len().max(1) as u64
            * 2
            * (req.test_timeout_ms().max(10_000) as u64 / 1000 + 1),
    )
}

fn speed_budget(req: &pb::SpeedTestRequest) -> Duration {
    Duration::from_secs(
        120 + req.outbound_tags.len().max(1) as u64
            * (60 + 2 * (req.timeout_ms().max(10_000) as u64 / 1000 + 1)),
    )
}
