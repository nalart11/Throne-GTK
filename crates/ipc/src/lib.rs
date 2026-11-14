//! Transport to the ThroneGtkCore core.
//!
//! The core does not create the endpoint itself: it connects to ours through a
//! Unix socket or Windows named pipe, checking that the peer is its own parent
//! and that the parent's binary is nearby and named `throne-gtk`.
//!
//! Frames (little-endian):
//!   request: [u32 req_id][u16 method_len][method][u32 payload_len][payload]
//!   response: [u32 req_id][u8 status][u32 data_len][data]
//! status != 0 — the response body is an error message.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

#[cfg(target_os = "macos")]
mod service_macos;

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

/// Maximum frame size. The core is trusted, but the channel still must not be
/// able to make us allocate an arbitrary amount of memory from a malformed header.
const MAX_FRAME: u32 = 64 * 1024 * 1024;

type Pending = Arc<Mutex<HashMap<u32, oneshot::Sender<Result<Vec<u8>>>>>>;

struct Call {
    id: u32,
    method: &'static str,
    payload: Vec<u8>,
}
struct Transport {
    pending: Pending,
    cancel: watch::Sender<bool>,
    slots: Semaphore,
    next_id: AtomicU32,
}
impl Transport {
    fn fail(&self) {
        self.cancel.send_replace(true);
        self.slots.close();
        for (_, reply) in self.pending.lock().unwrap().drain() {
            let _ = reply.send(Err(anyhow!("соединение с ядром разорвано")));
        }
    }
}
struct PendingGuard {
    pending: Pending,
    id: u32,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
struct EndpointGuard(PathBuf, Option<std::fs::Metadata>);
impl EndpointGuard {
    fn new(path: PathBuf) -> Self {
        let metadata = std::fs::symlink_metadata(&path).ok();
        Self(path, metadata)
    }
}
impl Drop for EndpointGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let (Some(owned), Ok(current)) = (&self.1, std::fs::symlink_metadata(&self.0)) {
                if owned.dev() == current.dev() && owned.ino() == current.ino() {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
        }
    }
}
static ENDPOINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
fn endpoint_name(dir: &Path) -> PathBuf {
    let seq = ENDPOINT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    #[cfg(unix)]
    return dir.join(format!("core-{}-{seq}.sock", std::process::id()));
    #[cfg(windows)]
    {
        let _ = dir;
        PathBuf::from(format!(r"\\.\pipe\throne-gtk-{}-{seq}", std::process::id()))
    }
}

fn endpoint_occupied(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::AlreadyExists
    ) {
        return true;
    }
    // First-instance named pipes report ACCESS_DENIED when another server
    // already owns that name (or PIPE_BUSY while its instance is connected).
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(5 | 231)) {
        return true;
    }
    false
}

/// Client for one live connection. Closing its Core invalidates every clone.
#[derive(Clone)]
pub struct CoreClient {
    tx: mpsc::Sender<Call>,
    state: Arc<Transport>,
}

pub struct Core {
    child: Option<Child>,
    endpoint: Option<EndpointGuard>,
    tasks: Tasks,
    closed: bool,
    close_error: Option<String>,
    pub client: CoreClient,
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
        #[cfg(unix)]
        std::fs::create_dir_all(socket_dir)
            .with_context(|| format!("создание {}", socket_dir.display()))?;
        let (listener, endpoint) = {
            let mut bound = None;
            for _ in 0..32 {
                let path = endpoint_name(socket_dir);
                #[cfg(target_os = "macos")]
                {
                    use std::os::unix::ffi::OsStrExt;
                    if path.as_os_str().as_bytes().len() >= 104 {
                        bail!("слишком длинный путь IPC: {}", path.display());
                    }
                }
                #[cfg(unix)]
                let result = UnixListener::bind(&path);
                #[cfg(windows)]
                let result = ServerOptions::new().first_pipe_instance(true).create(&path);
                match result {
                    Ok(listener) => {
                        bound = Some((listener, EndpointGuard::new(path)));
                        break;
                    }
                    Err(e) if endpoint_occupied(&e) => continue,
                    Err(e) => return Err(e).with_context(|| format!("bind {}", path.display())),
                }
            }
            bound.context("не удалось создать уникальный IPC endpoint")?
        };
        let mut tasks = Tasks(Vec::new());
        let mut child = None;
        if privileged {
            #[cfg(target_os = "macos")]
            if let Err(error) = tokio::time::timeout(
                Duration::from_secs(30),
                service_macos::start_privileged_core(&endpoint.0, debug),
            )
            .await
            .context("таймаут запуска привилегированного ядра")?
            {
                return Err(error);
            }
            #[cfg(not(target_os = "macos"))]
            unreachable!("privileged helper is only used on macOS");
        } else {
            let mut cmd = Command::new(core_bin);
            cmd.env("THRONE_CORE_SOCKET", &endpoint.0)
                .env("THRONE_CORE_DEBUG", if debug { "1" } else { "0" })
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
            child = Some(spawned);
        }

        // The core retries the connection ten times at 500 ms intervals; wait with some margin.
        #[cfg(unix)]
        let accept = async { listener.accept().await.map(|(stream, _)| stream) };
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
                     `throne-gtk` и лежит рядом с ядром",
                    endpoint.0.display()
                );
            }
        };

        let (client, transport_tasks) = CoreClient::attach(stream);
        // Move handles without letting the temporary task owner abort them.
        let mut transport_tasks = transport_tasks;
        tasks.0.append(&mut transport_tasks.0);
        Ok(Self {
            child,
            endpoint: Some(endpoint),
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
        self.client.state.fail();
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
        self.endpoint.take();
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
        self.client.state.fail();
        for task in &self.tasks.0 {
            task.abort();
        }
    }
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
    fn attach(stream: IpcStream) -> (Self, Tasks) {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::channel::<Call>(64);
        let (cancel, _) = watch::channel(false);
        let state = Arc::new(Transport {
            pending: Arc::default(),
            cancel,
            slots: Semaphore::new(64),
            next_id: AtomicU32::new(1),
        });
        let mut tasks = Tasks(Vec::new());
        let s = state.clone();
        let mut cancelled = s.cancel.subscribe();
        tasks.0.push(tokio::spawn(async move {
            let run = async {
                while let Some(call) = rx.recv().await {
                    if !s.pending.lock().unwrap().contains_key(&call.id) {
                        continue;
                    }
                    let m = call.method.as_bytes();
                    let mut frame = Vec::with_capacity(10 + m.len() + call.payload.len());
                    frame.extend_from_slice(&call.id.to_le_bytes());
                    frame.extend_from_slice(&(m.len() as u16).to_le_bytes());
                    frame.extend_from_slice(m);
                    frame.extend_from_slice(&(call.payload.len() as u32).to_le_bytes());
                    frame.extend_from_slice(&call.payload);
                    if !matches!(
                        tokio::time::timeout(Duration::from_secs(5), writer.write_all(&frame))
                            .await,
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                }
            };
            if !*cancelled.borrow() {
                tokio::select! { _ = run => {}, _ = cancelled.changed() => {} }
            }
            s.fail();
        }));
        let s = state.clone();
        let mut cancelled = s.cancel.subscribe();
        tasks.0.push(tokio::spawn(async move {
            let run = async {
                loop {
                    let mut head = [0u8; 9];
                    if reader.read_exact(&mut head).await.is_err() {
                        break;
                    }
                    let id = u32::from_le_bytes(head[..4].try_into().unwrap());
                    let len = u32::from_le_bytes(head[5..].try_into().unwrap());
                    if len > MAX_FRAME {
                        break;
                    }
                    let mut data = vec![0; len as usize];
                    if reader.read_exact(&mut data).await.is_err() {
                        break;
                    }
                    if let Some(reply) = s.pending.lock().unwrap().remove(&id) {
                        let _ = reply.send(if head[4] == 0 {
                            Ok(data)
                        } else {
                            Err(anyhow!(String::from_utf8_lossy(&data).into_owned()))
                        });
                    }
                }
            };
            if !*cancelled.borrow() {
                tokio::select! { _ = run => {}, _ = cancelled.changed() => {} }
            }
            s.fail();
        }));
        (Self { tx, state }, tasks)
    }

    async fn call_with_timeout<Req: Message, Resp: Message + Default>(
        &self,
        method: &'static str,
        req: Req,
        budget: Duration,
    ) -> Result<Resp> {
        tokio::time::timeout(budget, async {
            let _slot = self
                .state
                .slots
                .acquire()
                .await
                .context("ядро не запущено")?;
            let (reply, wait) = oneshot::channel();
            let id = {
                let mut pending = self.state.pending.lock().unwrap();
                if *self.state.cancel.borrow() {
                    bail!("ядро не запущено");
                }
                let id = loop {
                    let id = self.state.next_id.fetch_add(1, Ordering::Relaxed);
                    if !pending.contains_key(&id) {
                        break id;
                    }
                };
                pending.insert(id, reply);
                id
            };
            let _guard = PendingGuard {
                pending: self.state.pending.clone(),
                id,
            };
            let payload = req.encode_to_vec();
            if payload.len() > MAX_FRAME as usize {
                bail!("слишком большой IPC запрос");
            }
            self.tx
                .send(Call {
                    id,
                    method,
                    payload,
                })
                .await
                .context("ядро не запущено")?;
            let data = wait.await.context("ответ ядра потерян")??;
            Resp::decode(data.as_slice()).with_context(|| format!("разбор ответа {method}"))
        })
        .await
        .with_context(|| format!("таймаут {method}"))?
    }

    async fn call<Req: Message, Resp: Message + Default>(
        &self,
        method: &'static str,
        req: Req,
    ) -> Result<Resp> {
        let seconds = match method {
            "Start" | "CheckConfig" => 120,
            "IPTest" => 120,
            _ => 5,
        };
        self.call_with_timeout(method, req, Duration::from_secs(seconds))
            .await
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
        self.call("IPTest", req).await
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

fn latency_budget(req: &pb::TestReq) -> Duration {
    let c = req.max_concurrency();
    let c = if c <= 0 || c >= 500 { 100 } else { c as u64 };
    let n = req.outbound_tags.len().max(1) as u64;
    let probe = if req.test_timeout_ms() <= 0 {
        3000
    } else {
        req.test_timeout_ms() as u64
    };
    Duration::from_millis(30_000 + n.div_ceil(c).saturating_mul(2 * probe))
}

fn speed_budget(req: &pb::SpeedTestRequest) -> Duration {
    let n = req.outbound_tags.len().max(1) as u64;
    let phases = u64::from(req.test_download()) + u64::from(req.test_upload());
    let ms = req.timeout_ms().max(10_000) as u64;
    Duration::from_millis(30_000 + n.saturating_mul(8_000 + phases * ms))
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
        client.state.fail();
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
        let (client, tasks) = CoreClient::attach(stream);
        let c = client.clone();
        let call = tokio::spawn(async move {
            c.call_with_timeout::<_, pb::EmptyResp>(
                "Test",
                pb::EmptyReq {},
                Duration::from_millis(50),
            )
            .await
        });
        let id = request(&mut peer).await;
        assert!(call.await.unwrap().is_err());
        assert_eq!(client.state.pending.lock().unwrap().len(), 0);
        peer.write_all(&[id.to_le_bytes().as_slice(), &[0], &0u32.to_le_bytes()].concat())
            .await
            .unwrap();
        let c = client.clone();
        let call = tokio::spawn(async move { c.query_stats().await });
        request(&mut peer).await;
        call.abort();
        let _ = call.await;
        assert_eq!(client.state.pending.lock().unwrap().len(), 0);
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
        let (client, tasks) = CoreClient::attach(stream);
        let clone = client.clone();
        let mut core = Core {
            child: None,
            endpoint: None,
            tasks,
            closed: false,
            close_error: None,
            client,
        };
        assert!(core.close(Duration::from_millis(20)).await.is_err());
        assert!(core.tasks.0.is_empty());
        assert!(clone.query_stats().await.is_err());
        assert_eq!(clone.state.pending.lock().unwrap().len(), 0);
        assert!(core.close(Duration::from_secs(1)).await.is_err());
    }

    #[tokio::test]
    async fn writer_failure_wakes_all_and_joins() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let (client, tasks) = CoreClient::attach(stream);
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
                Duration::from_secs(10),
            )
            .await
        });
        assert!(tokio::time::timeout(Duration::from_secs(8), first)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(second.await.unwrap().is_err());
        assert_eq!(client.state.pending.lock().unwrap().len(), 0);
        finish(&client, tasks).await;
    }

    #[test]
    fn endpoint_guards_are_independent_and_preserve_replacements() {
        // Short path is required by Darwin sockaddr_un.
        let dir = PathBuf::from("/tmp");
        let a = endpoint_name(&dir);
        let b = endpoint_name(&dir);
        let _a_listener = std::os::unix::net::UnixListener::bind(&a).unwrap();
        let ga = EndpointGuard::new(a.clone());
        let _b_listener = std::os::unix::net::UnixListener::bind(&b).unwrap();
        let gb = EndpointGuard::new(b.clone());
        drop(ga);
        assert!(!a.exists());
        assert!(b.exists());
        std::fs::remove_file(&b).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&b).unwrap();
        drop(gb);
        assert!(b.exists());
        std::fs::remove_file(b).unwrap();
    }

    #[test]
    fn concurrent_endpoint_names_unique() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..100)
                        .map(|_| endpoint_name(Path::new("/tmp")))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let names: std::collections::HashSet<_> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        assert_eq!(names.len(), 800);
    }

    #[test]
    fn long_latency_deadline_arithmetic() {
        let mut req = pb::TestReq {
            outbound_tags: vec![String::new(); 501],
            ..Default::default()
        };
        assert_eq!(latency_budget(&req), Duration::from_secs(66));
        req.max_concurrency = Some(500);
        assert_eq!(latency_budget(&req), Duration::from_secs(66));
        req.max_concurrency = Some(2);
        req.test_timeout_ms = Some(10_000);
        assert_eq!(latency_budget(&req), Duration::from_secs(5050));
        let speed = pb::SpeedTestRequest {
            outbound_tags: vec![String::new(); 10],
            test_download: Some(true),
            test_upload: Some(true),
            timeout_ms: Some(60_000),
            ..Default::default()
        };
        assert_eq!(speed_budget(&speed), Duration::from_secs(1310));
    }
}
