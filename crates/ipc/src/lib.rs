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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

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
    method: &'static str,
    payload: Vec<u8>,
    reply: oneshot::Sender<Result<Vec<u8>>>,
}

/// Client for one live connection to the core. Can be cloned freely.
#[derive(Clone)]
pub struct CoreClient {
    tx: mpsc::Sender<Call>,
}

/// The running core process together with its channel.
pub struct Core {
    child: Child,
    endpoint: PathBuf,
    pub client: CoreClient,
}

impl Core {
    /// Creates the IPC endpoint, starts `core_bin`, and waits for it to connect.
    pub async fn spawn(core_bin: &Path, socket_dir: &Path, debug: bool) -> Result<Self> {
        // Windows derives a named-pipe name from the PID and has no socket directory.
        #[cfg(windows)]
        let _ = socket_dir;

        #[cfg(unix)]
        let endpoint = socket_dir.join(format!("core-{}.sock", std::process::id()));
        #[cfg(windows)]
        let endpoint = PathBuf::from(format!(r"\\.\pipe\throne-gtk-{}", std::process::id()));

        #[cfg(unix)]
        let listener = {
            cleanup_endpoint(&endpoint);
            std::fs::create_dir_all(socket_dir)
                .with_context(|| format!("создание {}", socket_dir.display()))?;
            UnixListener::bind(&endpoint).with_context(|| format!("bind {}", endpoint.display()))?
        };
        #[cfg(windows)]
        let listener = ServerOptions::new()
            // Refuse to attach to an endpoint planted before us. The core also
            // verifies the server PID after connecting.
            .first_pipe_instance(true)
            .create(&endpoint)
            .with_context(|| format!("создание {}", endpoint.display()))?;

        let mut cmd = Command::new(core_bin);
        cmd.env("THRONE_CORE_SOCKET", &endpoint)
            .env("THRONE_CORE_DEBUG", if debug { "1" } else { "0" })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                cleanup_endpoint(&endpoint);
                return Err(e).with_context(|| format!("запуск ядра {}", core_bin.display()));
            }
        };

        pipe_core_logs(&mut child);

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
                let _ = child.kill().await;
                cleanup_endpoint(&endpoint);
                return Err(e).context("accept от ядра");
            }
            Err(_) => {
                let _ = child.kill().await;
                cleanup_endpoint(&endpoint);
                bail!(
                    "ядро не подключилось к {} за 15 с — проверьте, что бинарь GUI называется \
                     `throne-gtk` и лежит рядом с ядром",
                    endpoint.display()
                );
            }
        };

        let client = CoreClient::attach(stream);
        Ok(Self {
            child,
            endpoint,
            client,
        })
    }

    pub async fn shutdown(mut self) {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), self.client.stop()).await;
        let _ = self.child.kill().await;
        cleanup_endpoint(&self.endpoint);
    }

    /// Whether the core process has exited on its own.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        cleanup_endpoint(&self.endpoint);
    }
}

#[cfg(unix)]
fn cleanup_endpoint(endpoint: &Path) {
    let _ = std::fs::remove_file(endpoint);
}

#[cfg(windows)]
fn cleanup_endpoint(_endpoint: &Path) {
    // A named pipe disappears when its final handle is closed.
}

fn pipe_core_logs(child: &mut Child) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "core", "{line}");
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "core", "{line}");
            }
        });
    }
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
    fn attach(stream: IpcStream) -> Self {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::channel::<Call>(64);
        let pending: Pending = Arc::default();
        let next_id = Arc::new(AtomicU32::new(1));

        // Writer: frames requests and remembers where to send each response.
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                while let Some(call) = rx.recv().await {
                    let id = next_id.fetch_add(1, Ordering::Relaxed);
                    pending.lock().await.insert(id, call.reply);

                    let m = call.method.as_bytes();
                    let mut frame = Vec::with_capacity(4 + 2 + m.len() + 4 + call.payload.len());
                    frame.extend_from_slice(&id.to_le_bytes());
                    frame.extend_from_slice(&(m.len() as u16).to_le_bytes());
                    frame.extend_from_slice(m);
                    frame.extend_from_slice(&(call.payload.len() as u32).to_le_bytes());
                    frame.extend_from_slice(&call.payload);

                    if writer.write_all(&frame).await.is_err() {
                        if let Some(reply) = pending.lock().await.remove(&id) {
                            let _ = reply.send(Err(anyhow!("соединение с ядром разорвано")));
                        }
                        break;
                    }
                }
            });
        }

        // Reader: parses responses and wakes the waiters by req_id.
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                loop {
                    let mut head = [0u8; 9];
                    if reader.read_exact(&mut head).await.is_err() {
                        break;
                    }
                    let id = u32::from_le_bytes(head[0..4].try_into().unwrap());
                    let status = head[4];
                    let len = u32::from_le_bytes(head[5..9].try_into().unwrap());
                    if len > MAX_FRAME {
                        break;
                    }
                    let mut data = vec![0u8; len as usize];
                    if len > 0 && reader.read_exact(&mut data).await.is_err() {
                        break;
                    }
                    if let Some(reply) = pending.lock().await.remove(&id) {
                        let _ = reply.send(if status == 0 {
                            Ok(data)
                        } else {
                            Err(anyhow!(String::from_utf8_lossy(&data).into_owned()))
                        });
                    }
                }
                // The connection closed — none of the response waiters will receive one.
                for (_, reply) in pending.lock().await.drain() {
                    let _ = reply.send(Err(anyhow!("ядро закрыло соединение")));
                }
            });
        }

        Self { tx }
    }

    async fn call<Req: Message, Resp: Message + Default>(
        &self,
        method: &'static str,
        req: Req,
    ) -> Result<Resp> {
        let (reply, wait) = oneshot::channel();
        self.tx
            .send(Call {
                method,
                payload: req.encode_to_vec(),
                reply,
            })
            .await
            .map_err(|_| anyhow!("ядро не запущено"))?;
        let data = wait.await.map_err(|_| anyhow!("ответ ядра потерян"))??;
        Resp::decode(data.as_slice()).with_context(|| format!("разбор ответа {method}"))
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
        self.call("Test", req).await
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
        self.call("SpeedTest", req).await
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
