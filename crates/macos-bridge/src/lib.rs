//! Isolated, blocking C ABI. Call from a non-UI thread.
#![cfg(target_os = "macos")]

#[path = "../../app/src/engine.rs"]
mod engine;
#[path = "../../app/src/i18n.rs"]
mod i18n;
#[path = "../../app/src/logbridge.rs"]
mod logbridge;
#[path = "../../app/src/paths.rs"]
mod paths;

use anyhow::{anyhow, bail, Context, Result};
use engine::{Command, Engine, Event, Status};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    ffi::{c_char, CString},
    path::PathBuf,
    sync::{mpsc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use throne_config::{profile::Group, Profile, Settings};
use throne_store::Store;

const INPUT_CAP: usize = 4 * 1024 * 1024;
type Reply = mpsc::Sender<Result<Value>>;
enum Message {
    Request(Value, Reply),
    Poll(u32, Reply),
    Close(u64, Reply),
}
struct Registry {
    next: u64,
    // None is confirmed shutdown awaiting acknowledgement, in the original
    // handle slot (no separate, growing tombstone collection).
    workers: HashMap<u64, Option<mpsc::SyncSender<Message>>>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next: 1,
            workers: HashMap::new(),
        })
    })
}
// One native Store/engine owner at a time; closed handles remain registry-validated.
static ACTIVE: Mutex<bool> = Mutex::new(false);
static LOCALE: OnceLock<()> = OnceLock::new();
static LOG_SINK: Mutex<Option<async_channel::Sender<Event>>> = Mutex::new(None);
static LOG_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();
fn logs_for_worker() -> Result<async_channel::Receiver<Event>> {
    use tracing_subscriber::prelude::*;
    let initialized = LOG_INIT.get_or_init(|| {
        let (tx, rx) = async_channel::bounded(256);
        tracing_subscriber::registry()
            .with(logbridge::LogBridge::new(tx))
            .try_init()
            .map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("throne-log-router".into())
            .spawn(move || {
                while let Ok(event) = rx.recv_blocking() {
                    if let Ok(sink) = LOG_SINK.lock() {
                        if let Some(tx) = sink.as_ref() {
                            let _ = tx.try_send(event);
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    });
    initialized
        .as_ref()
        .map_err(|e| anyhow!("log subscriber unavailable: {e}"))?;
    let (tx, rx) = async_channel::bounded(256);
    *LOG_SINK
        .lock()
        .map_err(|_| anyhow!("log router poisoned"))? = Some(tx);
    Ok(rx)
}

unsafe fn input(p: *const c_char) -> Result<Value> {
    if p.is_null() {
        bail!("null JSON input");
    }
    let mut n = 0;
    while n <= INPUT_CAP {
        if *p.add(n) == 0 {
            return Ok(serde_json::from_slice(std::slice::from_raw_parts(
                p as *const u8,
                n,
            ))?);
        }
        n += 1;
    }
    bail!("JSON input exceeds 4 MiB")
}
fn boundary(f: impl FnOnce() -> Result<Value>) -> *mut c_char {
    let value = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => json!({"ok":true,"value":v}),
        Ok(Err(e)) => json!({"ok":false,"error":format!("{e:#}")}),
        Err(_) => json!({"ok":false,"error":"bridge panic"}),
    };
    CString::new(value.to_string())
        .expect("JSON has no NUL")
        .into_raw()
}
#[no_mangle]
pub unsafe extern "C" fn throne_macos_open(p: *const c_char) -> *mut c_char {
    boundary(|| open(input(p)?))
}
#[no_mangle]
pub unsafe extern "C" fn throne_macos_request(h: u64, p: *const c_char) -> *mut c_char {
    boundary(|| call(h, |r| Ok(Message::Request(input(p)?, r))))
}
#[no_mangle]
pub extern "C" fn throne_macos_poll(h: u64, ms: u32) -> *mut c_char {
    boundary(|| call(h, |r| Ok(Message::Poll(ms.min(1000), r))))
}
#[no_mangle]
pub extern "C" fn throne_macos_close(h: u64) -> *mut c_char {
    boundary(|| close_with_timeout(h, Duration::from_secs(7)))
}
fn close_with_timeout(h: u64, timeout: Duration) -> Result<Value> {
    let tx = {
        let mut reg = registry()
            .lock()
            .map_err(|_| anyhow!("registry poisoned"))?;
        match reg.workers.get(&h) {
            Some(Some(tx)) => tx.clone(),
            Some(None) => {
                reg.workers.remove(&h);
                return Ok(Value::Null);
            }
            None => bail!("invalid or closed handle"),
        }
    };
    let (reply, rx) = mpsc::channel();
    tx.try_send(Message::Close(h, reply))
        .map_err(|e| anyhow!("close queue unavailable; retry close: {e}"))?;
    let result = rx
        .recv_timeout(timeout)
        .context("bridge close timed out; engine termination not guaranteed")??;
    registry()
        .lock()
        .map_err(|_| anyhow!("registry poisoned"))?
        .workers
        .remove(&h);
    Ok(result)
}
fn record_closed(h: u64) -> Result<()> {
    let mut reg = registry()
        .lock()
        .map_err(|_| anyhow!("registry poisoned"))?;
    if let Some(slot) = reg.workers.get_mut(&h) {
        *slot = None;
    }
    Ok(())
}
#[no_mangle]
pub unsafe extern "C" fn throne_macos_string_free(p: *mut c_char) {
    if !p.is_null() {
        drop(CString::from_raw(p));
    }
}
fn call(h: u64, f: impl FnOnce(Reply) -> Result<Message>) -> Result<Value> {
    let tx = registry()
        .lock()
        .map_err(|_| anyhow!("registry poisoned"))?
        .workers
        .get(&h)
        .and_then(|slot| slot.clone())
        .context("invalid or closed handle")?;
    let (reply, rx) = mpsc::channel();
    tx.try_send(f(reply)?)
        .map_err(|e| anyhow!("bridge queue unavailable: {e}"))?;
    rx.recv_timeout(Duration::from_secs(35))
        .context("bridge request timed out")?
}
fn open(options: Value) -> Result<Value> {
    let mut active = ACTIVE
        .lock()
        .map_err(|_| anyhow!("engine guard poisoned"))?;
    if *active {
        bail!("another bridge engine is active or its shutdown is unconfirmed");
    }
    let dir = options
        .get("data_dir")
        .map(|v| {
            v.as_str()
                .map(PathBuf::from)
                .context("data_dir must be a string")
        })
        .transpose()?
        .unwrap_or_else(paths::config_dir);
    let core = options
        .get("core_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(paths::core_binary);
    let (tx, rx) = mpsc::sync_channel(64);
    let (ready, initialized) = mpsc::channel();
    *active = true;
    if let Err(e) = std::thread::Builder::new()
        .name("throne-macos-store".into())
        .spawn(move || {
            let mut state = match State::new(dir, core) {
                Ok(s) => {
                    let _ = ready.send(Ok(()));
                    s
                }
                Err(e) => {
                    let _ = ready.send(Err(format!("{e:#}")));
                    if let Ok(mut a) = ACTIVE.lock() {
                        *a = false;
                    }
                    return;
                }
            };
            loop {
                state.drain();
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(Message::Request(v, r)) => {
                        let mutates =
                            !matches!(v["op"].as_str(), Some("snapshot" | "export_config"));
                        let result = state.request(v);
                        if result.is_ok() && mutates {
                            state.changed = true;
                        }
                        let _ = r.send(result);
                    }
                    Ok(Message::Poll(ms, r)) => {
                        let until = Instant::now() + Duration::from_millis(ms as u64);
                        while !state.changed && Instant::now() < until {
                            state.drain();
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        let v = if std::mem::take(&mut state.changed) {
                            json!({"kind":"changed"})
                        } else {
                            Value::Null
                        };
                        let _ = r.send(Ok(v));
                    }
                    Ok(Message::Close(h, r)) => {
                        let result = state.shutdown();
                        let clean = result.is_ok();
                        if clean {
                            // Record before replying: a timed-out caller may have
                            // dropped its receiver while this Close was queued.
                            if let Err(e) = record_closed(h) {
                                let _ = r.send(Err(e));
                                break;
                            }
                            if let Ok(mut a) = ACTIVE.lock() {
                                *a = false;
                            }
                            let _ = r.send(result);
                            break;
                        }
                        let _ = r.send(result);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => {
                        if state.shutdown().is_ok() {
                            if let Ok(mut a) = ACTIVE.lock() {
                                *a = false;
                            }
                        }
                        break;
                    }
                }
            }
        })
    {
        *active = false;
        return Err(e.into());
    }
    drop(active);
    initialized.recv()?.map_err(|e| anyhow!(e))?;
    let mut reg = registry()
        .lock()
        .map_err(|_| anyhow!("registry poisoned"))?;
    let h = reg.next;
    reg.next = reg.next.checked_add(1).context("handle exhaustion")?;
    reg.workers.insert(h, Some(tx));
    Ok(json!(h))
}
struct State {
    runtime: tempfile::TempDir,
    log_events: async_channel::Receiver<Event>,
    closing: bool,
    store: Store,
    settings: Settings,
    engine: Engine,
    events: async_channel::Receiver<Event>,
    status: Status,
    selected: i64,
    logs: VecDeque<String>,
    connections: Value,
    traffic: [i64; 4],
    auto: Value,
    testing: bool,
    speed: String,
    latency: HashMap<i64, String>,
    latency_error: Option<String>,
    speed_profile: Option<(i64, String)>,
    checking: bool,
    subscription: Option<i64>,
    notice: Option<String>,
    changed: bool,
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
impl State {
    fn localized<'a>(&self, ru: &'a str, en: &'a str) -> &'a str {
        match self.settings.language {
            throne_config::settings::Language::Ru => ru,
            _ => en,
        }
    }
    fn clear_selection(&mut self) -> Result<()> {
        self.selected = 0;
        self.settings.last_profile_id = 0;
        self.store.save_settings(&self.settings)
    }
    fn new(dir: PathBuf, core: PathBuf) -> Result<Self> {
        let store = Store::open(&dir.join("throne-gtk.db"))?;
        let mut settings = store.settings()?;
        settings.cache_file = dir.join("cache.db").to_string_lossy().into_owned();
        LOCALE.get_or_init(|| i18n::init(settings.language));
        let (tx, events) = async_channel::bounded(256);
        let log_events = logs_for_worker()?;
        // Prefer the approved per-user TMPDIR. Darwin sockaddr_un.sun_path has
        // 104 bytes including NUL: reserve the random suffix and core socket.
        let preferred = std::env::temp_dir();
        use std::os::unix::ffi::OsStrExt;
        let socket_suffix = format!("/tn-XXXXXX/core-{}-{}.sock", std::process::id(), u64::MAX);
        let base = if preferred.as_os_str().as_bytes().len() + socket_suffix.len() + 1 < 104 {
            preferred
        } else {
            PathBuf::from("/tmp")
        };
        let runtime = tempfile::Builder::new().prefix("tn-").tempdir_in(base)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700))?;
        let engine = Engine::start(core, runtime.path().to_owned(), tx);
        Ok(Self {
            runtime,
            log_events,
            closing: false,
            selected: settings.last_profile_id,
            store,
            settings,
            engine,
            events,
            status: Status::Disconnected,
            logs: VecDeque::new(),
            connections: json!([]),
            traffic: [0; 4],
            auto: Value::Null,
            testing: false,
            speed: String::new(),
            latency: HashMap::new(),
            latency_error: None,
            speed_profile: None,
            checking: false,
            subscription: None,
            notice: None,
            changed: true,
        })
    }
    fn shutdown(&mut self) -> Result<Value> {
        self.shutdown_timeout(Duration::from_secs(5))
    }
    fn shutdown_timeout(&mut self, timeout: Duration) -> Result<Value> {
        self.closing = true;
        let result = self.engine.shutdown(timeout);
        self.drain();
        result?;
        Ok(Value::Null)
    }
    fn drain(&mut self) {
        for _ in 0..256 {
            match self.events.try_recv() {
                Ok(e) => self.reduce_report(e),
                Err(_) => break,
            }
        }
        // Logs cannot starve commands or grow without bounds.
        for _ in 0..256 {
            match self.log_events.try_recv() {
                Ok(e) => self.reduce_report(e),
                Err(_) => break,
            }
        }
    }
    fn reduce_report(&mut self, e: Event) {
        if let Err(e) = self.reduce(e) {
            self.notice = Some(format!("{e:#}"));
        }
        self.changed = true;
    }
    fn profile(&self, id: i64) -> Result<Profile> {
        self.store.profile(id)?.context("profile not found")
    }
    fn mutable(&self) -> Result<()> {
        if self.testing || self.speed_profile.is_some() || self.checking {
            bail!("profile/configuration measurement in progress");
        }
        if matches!(self.status, Status::Connected { .. } | Status::Connecting) {
            bail!("disconnect before changing connection profiles");
        }
        if self.subscription.is_some() {
            bail!("subscription refresh in progress");
        }
        Ok(())
    }
    fn snapshot(&self) -> Result<Value> {
        let status = match &self.status {
            Status::Disconnected => json!({"kind":"disconnected"}),
            Status::Connecting => json!({"kind":"connecting"}),
            Status::Connected { profile_id, name } => {
                json!({"kind":"connected","profile_id":profile_id,"name":name})
            }
            Status::Failed(e) => json!({"kind":"failed","error":e}),
        };
        Ok(
            json!({"groups":self.store.groups()?.into_iter().map(GroupWire).collect::<Vec<_>>(),"profiles":self.store.profiles(None)?.into_iter().map(ProfileWire).collect::<Vec<_>>(),"settings":self.settings,"status":status,"selected_id":self.selected,"logs":self.logs,"connections":self.connections,"traffic":{"down":self.traffic[0],"up":self.traffic[1],"total_down":self.traffic[2],"total_up":self.traffic[3]},"auto_status":self.auto,"testing":self.testing,"speed_stage":self.speed,"notice":self.notice}),
        )
    }
    fn request(&mut self, v: Value) -> Result<Value> {
        let op = v.get("op").and_then(Value::as_str).context("missing op")?;
        if self.closing && !matches!(op, "snapshot" | "logs_clear") {
            bail!("shutdown pending; retry close before further commands");
        }
        let id = || {
            v.get("id")
                .and_then(Value::as_i64)
                .context("missing integer id")
        };
        let gid = || {
            v.get("gid")
                .and_then(Value::as_i64)
                .context("missing integer gid")
        };
        match op {
            "snapshot" => return self.snapshot(),
            "select" => {
                let n = id()?;
                if n < 0 {
                    self.store
                        .group(n.checked_neg().context("invalid group")?)?
                        .context("group not found")?;
                } else {
                    self.profile(n)?;
                }
                self.selected = n;
                self.settings.last_profile_id = n;
                self.store.save_settings(&self.settings)?;
            }
            "settings_patch" => {
                let mut merged = serde_json::to_value(&self.settings)?;
                let patch = v["patch"].as_object().context("patch must be an object")?;
                for (k, val) in patch {
                    if k == "cache_file" || merged.get(k).is_none() {
                        bail!("unknown or runtime-only setting: {k}");
                    }
                    merged[k] = val.clone();
                }
                let mut s: Settings = serde_json::from_value(merged)?;
                s.cache_file = self.settings.cache_file.clone();
                self.store.save_settings(&s)?;
                self.settings = s;
            }
            "group_save" => {
                self.mutable()?;
                let g: Group = serde_json::from_value::<GroupWire>(v["group"].clone())?.0;
                if g.id == 0 {
                    return Ok(json!({"id":self.store.insert_group(&g)?}));
                }
                self.store.group(g.id)?.context("group not found")?;
                self.store.update_group(&g)?;
            }
            "group_delete" => {
                self.mutable()?;
                self.store.group(id()?)?.context("group not found")?;
                self.store.delete_group(id()?)?;
                self.clear_selection()?;
            }
            "profile_save" => {
                self.mutable()?;
                let p: Profile = serde_json::from_value::<ProfileWire>(v["profile"].clone())?.0;
                self.store.group(p.gid)?.context("group not found")?;
                if !p.outbound.is_object() {
                    bail!("outbound must be an object");
                }
                if p.id == 0 {
                    return Ok(json!({"id":self.store.insert_profile(&p)?}));
                }
                self.profile(p.id)?;
                self.store.update_profile(&p)?;
            }
            "profile_delete" => {
                self.mutable()?;
                let ids = ids(&v)?;
                for &n in &ids {
                    self.profile(n)?;
                }
                self.store.delete_profiles(&ids)?;
                if ids.contains(&self.selected) {
                    self.clear_selection()?;
                }
            }
            "import_links" => {
                self.mutable()?;
                let g = gid()?;
                self.store.group(g)?.context("group not found")?;
                let parsed = throne_config::subscription::parse(
                    v["text"].as_str().context("missing text")?,
                )?;
                let mut n = 0;
                for mut p in parsed.profiles {
                    p.gid = g;
                    self.store.insert_profile(&p)?;
                    n += 1;
                }
                return Ok(json!({"profiles":n,"errors":parsed.errors}));
            }
            "import_throne" | "import_routes" => {
                self.mutable()?;
                let path = std::path::Path::new(v["path"].as_str().context("missing path")?);
                let result = if op == "import_throne" {
                    let n = self.store.import_throne(path)?;
                    json!({"groups":n.groups,"profiles":n.profiles,"rules":n.rules,"skipped_rules":n.skipped_rules})
                } else {
                    let (a, b) = self.store.import_throne_routes(path)?;
                    json!({"rules":a,"skipped_rules":b})
                };
                let cache = self.settings.cache_file.clone();
                self.settings = self.store.settings()?;
                self.settings.cache_file = cache;
                return Ok(result);
            }
            "connect" => {
                if self.subscription.is_some() {
                    bail!("subscription refresh in progress");
                }
                if self.settings.mode.is_vpn() && v["acknowledge_vpn"].as_bool() != Some(true) {
                    bail!("VPN requires explicit per-user action acknowledgement (acknowledge_vpn: true)");
                }
                let n = v.get("id").and_then(Value::as_i64).unwrap_or(self.selected);
                let settings = Box::new(self.settings.clone());
                let command = if n < 0 {
                    let g = n.checked_neg().context("invalid group")?;
                    self.store.group(g)?.context("group not found")?;
                    let mut profiles = self.store.profiles(Some(g))?;
                    profiles.sort_by_key(|p| match p.latency {
                        0 => i32::MAX - 1,
                        n if n < 0 => i32::MAX,
                        n => n,
                    });
                    if profiles.is_empty() {
                        bail!("group has no profiles");
                    }
                    Command::ConnectAuto {
                        profiles,
                        gid: g,
                        settings,
                    }
                } else {
                    Command::Connect {
                        profile: Box::new(self.profile(n)?),
                        settings,
                    }
                };
                self.selected = n;
                self.settings.last_profile_id = n;
                self.store.save_settings(&self.settings)?;
                self.engine.try_send(command)?;
                self.status = Status::Connecting;
            }
            "disconnect" => {
                self.engine.try_send(Command::Disconnect)?;
            }
            "test_latency" | "speed_test" | "check_config" => {
                if op == "test_latency" && self.testing
                    || op == "speed_test" && self.speed_profile.is_some()
                    || op == "check_config" && self.checking
                {
                    bail!("measurement already running");
                }
                if self.subscription.is_some() {
                    bail!("subscription refresh in progress");
                }
                let settings = Box::new(self.settings.clone());
                let command = match op {
                    "test_latency" => {
                        let mut ids = ids(&v)?;
                        if ids.is_empty() {
                            ids = self
                                .store
                                .profiles(Some(gid()?))?
                                .iter()
                                .map(|p| p.id)
                                .collect();
                        }
                        let profiles = ids
                            .iter()
                            .map(|&n| self.profile(n))
                            .collect::<Result<Vec<_>>>()?;
                        self.latency = profiles.iter().map(|p| (p.id, fingerprint(p))).collect();
                        Command::TestLatency {
                            profiles,
                            ids,
                            settings,
                        }
                    }
                    "speed_test" => Command::SpeedTest {
                        profile: Box::new(self.profile(id()?)?),
                        id: id()?,
                        settings,
                    },
                    _ => Command::CheckConfig {
                        profile: Box::new(self.profile(id()?)?),
                        settings,
                    },
                };
                self.engine.try_send(command)?;
                match op {
                    "test_latency" => {self.testing = true;self.latency_error=None;},
                    "speed_test" => {
                        let p = self.profile(id()?)?;
                        self.speed_profile = Some((p.id, fingerprint(&p)));
                        self.speed = self.localized("замеряю…", "measuring…").into();
                    }
                    _ => self.checking = true,
                }
            }
            "stop_test" => {
                self.engine.try_send(Command::StopTest)?;
            }
            "subscription_refresh" => {
                self.mutable()?;
                let g = self.store.group(gid()?)?.context("group not found")?;
                if g.url.is_empty() {
                    bail!("group has no subscription URL");
                }
                self.engine.try_send(Command::FetchSubscription {
                    gid: g.id,
                    url: g.url,
                    user_agent: self.settings.sub_user_agent.clone(),
                    send_hwid: self.settings.sub_send_hwid,
                    custom_hwid_params: self.settings.sub_custom_hwid_params.clone(),
                })?;
                self.subscription = Some(g.id);
            }
            "auto_recheck" | "auto_pin" => {
                if !matches!(self.status,Status::Connected{profile_id,..} if profile_id<0) {
                    bail!("auto connection required");
                }
                self.engine.try_send(if op == "auto_recheck" {
                    Command::AutoRecheck
                } else {
                    Command::AutoPin(v["name"].as_str().context("missing name")?.into())
                })?;
            }
            "logs_clear" => {
                self.logs.clear();
                self.notice = None;
            }
            "export_config" => {
                let generated = throne_config::generate(&self.profile(id()?)?, &self.settings)?;
                return Ok(json!(generated.core_config));
            }
            _ => bail!("unknown operation: {op}"),
        }
        Ok(Value::Null)
    }
    fn reduce(&mut self, e: Event) -> Result<()> {
        match e {
        Event::Status(s)=>{if matches!(s,Status::Disconnected|Status::Failed(_)){self.connections=json!([]);self.auto=Value::Null;self.traffic[0]=0;self.traffic[1]=0;}if let Status::Failed(ref e)=s{self.notice=Some(e.clone());}self.status=s;},
        Event::Traffic{down,up}=>{self.traffic[0]=down;self.traffic[1]=up;self.traffic[2]=self.traffic[2].saturating_add(down);self.traffic[3]=self.traffic[3].saturating_add(up);if let Status::Connected{profile_id,..}=self.status{if profile_id>0{self.store.add_traffic(profile_id,down,up)?;}}},
        Event::Connections(c)=>self.connections=Value::Array(c.into_iter().map(|c|json!({"id":c.id,"dest":c.dest,"domain":c.domain,"network":c.network,"process":c.process,"upload":c.upload,"download":c.download,"created_at":c.created_at})).collect()),
        Event::AutoStatus{selected,pinned,alive,total,suspended}=>self.auto=json!({"selected":selected,"pinned":pinned,"alive":alive,"total":total,"suspended":suspended}),
        Event::LatencyResult{id,latency}=>{let p=self.profile(id)?;if !self.testing || self.latency.get(&id)!=Some(&fingerprint(&p)){bail!("stale latency result");}self.store.set_latency(id,latency,now())?;},
        Event::TestFinished{tested}=>{if !self.testing{bail!("stale latency completion");}self.testing=false;self.latency.clear();let summary=match self.settings.language {throne_config::settings::Language::Ru=>format!("Проверено: {tested}"),_=>format!("Tested: {tested}")};self.notice=Some(match self.latency_error.take(){Some(error)=>format!("{summary}: {error}"),None=>summary});},
        Event::SpeedProgress{id,stage}=>{let p=self.profile(id)?;if self.speed_profile.as_ref()!=Some(&(id,fingerprint(&p))){bail!("stale speed progress");}self.speed=stage;},
        Event::SpeedResult{id,download,upload,country,latency,error}=>{let expected=self.speed_profile.take().context("stale speed completion")?;self.speed.clear();if expected.0!=id{self.speed_profile=Some(expected);bail!("stale speed profile ID");}if !error.is_empty(){bail!("{error}");}let p=self.profile(id)?;if fingerprint(&p)!=expected.1{bail!("stale speed result");}self.store.set_speed(id,&download,&upload,&country)?;if latency>0{self.store.set_latency(id,latency,now())?;}},
        Event::ConfigChecked(r)=>{if !self.checking{bail!("stale configuration completion");}self.checking=false;r.map_err(|e|anyhow!(e))?;self.notice=Some(self.localized("Конфигурация верна", "Configuration OK").into());},
        Event::Subscription{gid,result}=>{if self.subscription!=Some(gid){bail!("stale subscription result");}self.subscription=None;let parsed=result.map_err(|e|anyhow!(e))?;if parsed.profiles.is_empty(){bail!("subscription contains no profiles: {}",parsed.errors.join("\n"));}let mut g=self.store.group(gid)?.context("group no longer exists")?;let counts=self.store.replace_group_profiles(gid,&parsed.profiles)?;g.info=parsed.info;g.sub_last_update=now();self.store.update_group(&g)?;self.clear_selection()?;self.notice=Some(format!("{}: +{}, ={}, −{}{}",self.localized("Подписка", "Subscription"),counts.0,counts.1,counts.2,if parsed.errors.is_empty(){String::new()}else{format!("; {}",parsed.errors.join("; "))}));},
        Event::Log(line)=>{self.logs.push_back(line.chars().take(8192).collect());while self.logs.len()>500{self.logs.pop_front();}},
        Event::Error(e)=>{if self.testing{self.latency_error=Some(e.clone());}bail!("{e}");},
    }
        Ok(())
    }
}
fn ids(v: &Value) -> Result<Vec<i64>> {
    v["ids"]
        .as_array()
        .context("missing ids array")?
        .iter()
        .map(|v| v.as_i64().context("ids must be integers"))
        .collect()
}
fn fingerprint(profile: &Profile) -> String {
    json!([profile.gid, profile.kind, profile.outbound]).to_string()
}

// Existing models deliberately have no serde derives. These are wire adapters,
// not alternate business models; field names mirror their public Rust fields.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(remote = "Profile", default)]
struct ProfileDef {
    id: i64,
    kind: String,
    name: String,
    gid: i64,
    latency: i32,
    dl_speed: String,
    ul_speed: String,
    test_country: String,
    ip_out: String,
    outbound: Value,
    traffic_dl: i64,
    traffic_up: i64,
    created_at: i64,
    updated_at: i64,
    latency_at: i64,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct ProfileWire(#[serde(with = "ProfileDef")] Profile);
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(remote = "Group", default)]
struct GroupDef {
    id: i64,
    name: String,
    url: String,
    info: String,
    archive: bool,
    skip_auto_update: bool,
    sub_last_update: i64,
    created_at: i64,
    updated_at: i64,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct GroupWire(#[serde(with = "GroupDef")] Group);

#[cfg(test)]
mod tests {
    use super::*;
    unsafe fn decode(p: *mut c_char) -> Value {
        let v = serde_json::from_slice(std::ffi::CStr::from_ptr(p).to_bytes()).unwrap();
        throne_macos_string_free(p);
        v
    }
    unsafe fn request(h: u64, v: Value) -> Value {
        let s = CString::new(v.to_string()).unwrap();
        decode(throne_macos_request(h, s.as_ptr()))
    }
    #[test]
    fn bounded_queue_and_panic_boundary() {
        unsafe {
            let (tx, _rx) = mpsc::sync_channel(1);
            let (reply, _reply_rx) = mpsc::channel();
            tx.send(Message::Poll(0, reply)).unwrap();
            let h = {
                let mut reg = registry().lock().unwrap();
                let h = reg.next;
                reg.next += 1;
                reg.workers.insert(h, Some(tx));
                h
            };
            let result = request(h, json!({"op":"snapshot"}));
            assert_eq!(result["ok"], false);
            assert!(result["error"].as_str().unwrap().contains("queue"));
            assert_eq!(decode(throne_macos_close(h))["ok"], false);
            assert_eq!(request(h, json!({"op":"snapshot"}))["ok"], false);
            registry().lock().unwrap().workers.remove(&h);
            assert_eq!(decode(boundary(|| panic!("contained panic")))["ok"], false);
        }
    }
    #[test]
    fn late_success_close_retry_acknowledges_original_slot() {
        let (tx, rx) = mpsc::sync_channel(4);
        let h = {
            let mut reg = registry().lock().unwrap();
            let h = reg.next;
            reg.next += 1;
            reg.workers.insert(h, Some(tx));
            h
        };
        let (release, wait) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let Message::Close(handle, reply) = rx.recv().unwrap() else {
                panic!("expected close")
            };
            assert_eq!(handle, h);
            wait.recv().unwrap(); // Hold completion until the caller abandoned rx.
            record_closed(handle).unwrap(); // Same operation as confirmed worker shutdown.
            assert!(
                reply.send(Ok(Value::Null)).is_err(),
                "caller must already have timed out"
            );
        });
        assert!(close_with_timeout(h, Duration::from_millis(1))
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        assert!(
            matches!(registry().lock().unwrap().workers.get(&h), Some(Some(_))),
            "timeout must not claim shutdown success"
        );
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(
            matches!(registry().lock().unwrap().workers.get(&h), Some(None)),
            "late success must reuse original handle slot"
        );
        assert!(
            call(h, |r| Ok(Message::Request(json!({"op":"snapshot"}), r))).is_err(),
            "completed handles cannot serve requests"
        );
        unsafe {
            assert_eq!(
                decode(throne_macos_close(h))["ok"],
                true,
                "same-handle retry acknowledges confirmed late success"
            );
            assert_eq!(
                decode(throne_macos_close(h))["ok"],
                false,
                "acknowledgement consumes terminal slot"
            );
            assert_eq!(
                decode(throne_macos_close(u64::MAX))["ok"],
                false,
                "unknown handles cannot acknowledge success"
            );
        }
        assert!(!registry().lock().unwrap().workers.contains_key(&h));
    }
    #[test]
    fn failed_close_retains_handle_for_retry() {
        unsafe {
            let (tx, rx) = mpsc::sync_channel(4);
            let h = {
                let mut reg = registry().lock().unwrap();
                let h = reg.next;
                reg.next += 1;
                reg.workers.insert(h, Some(tx));
                h
            };
            let worker = std::thread::spawn(move || {
                if let Message::Close(_, r) = rx.recv().unwrap() {
                    r.send(Err(anyhow!("controlled shutdown timeout"))).unwrap();
                } else {
                    panic!()
                }
                if let Message::Request(_, r) = rx.recv().unwrap() {
                    r.send(Ok(json!({"still_open":true}))).unwrap();
                } else {
                    panic!()
                }
                if let Message::Close(_, r) = rx.recv().unwrap() {
                    r.send(Ok(Value::Null)).unwrap();
                } else {
                    panic!()
                }
            });
            assert_eq!(decode(throne_macos_close(h))["ok"], false);
            assert_eq!(
                request(h, json!({"op":"snapshot"}))["value"]["still_open"],
                true
            );
            assert_eq!(decode(throne_macos_close(h))["ok"], true);
            assert_eq!(request(h, json!({"op":"snapshot"}))["ok"], false);
            worker.join().unwrap();
        }
    }
    #[test]
    fn abi_store_lifecycle_and_reduction() {
        unsafe {
            let dir = tempfile::tempdir().unwrap();
            let options = CString::new(
                json!({"data_dir":dir.path(),"core_path":"/nonexistent-core"}).to_string(),
            )
            .unwrap();
            assert_eq!(decode(throne_macos_open(std::ptr::null()))["ok"], false);
            let opened = decode(throne_macos_open(options.as_ptr()));
            assert_eq!(opened["ok"], true, "{opened}");
            let h = opened["value"].as_u64().unwrap();
            let _ = decode(throne_macos_poll(h, 0));
            assert_eq!(request(h, json!({"op":"snapshot"}))["ok"], true);
            assert_eq!(
                decode(throne_macos_poll(h, 0))["value"],
                Value::Null,
                "snapshot must not dirty state"
            );
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    std::thread::spawn(move || {
                        for _ in 0..12 {
                            assert_eq!(request(h, json!({"op":"snapshot"}))["ok"], true);
                        }
                    })
                })
                .collect();
            for thread in threads {
                thread.join().unwrap();
            }
            assert_eq!(
                request(h, json!({"op":"snapshot"}))["value"]["settings"]["language"],
                "ru"
            );
            assert_eq!(
                request(
                    h,
                    json!({"op":"settings_patch","patch":{"language":"en","mixed_port":2345}})
                )["ok"],
                true
            );
            assert_eq!(
                request(
                    h,
                    json!({"op":"settings_patch","patch":{"cache_file":"bad"}})
                )["ok"],
                false
            );
            let group = request(h, json!({"op":"group_save","group":{"name":"Test"}}));
            let gid = group["value"]["id"].as_i64().unwrap();
            assert_eq!(
                request(
                    h,
                    json!({"op":"import_links","gid":gid,"text":"trojan://pw@a.example:443#a"})
                )["ok"],
                true
            );
            let snap = request(h, json!({"op":"snapshot"}));
            let id = snap["value"]["profiles"][0]["id"].as_i64().unwrap();
            let mut profile = snap["value"]["profiles"][0].clone();
            profile["name"] = json!("Edited");
            assert_eq!(
                request(h, json!({"op":"profile_save","profile":profile}))["ok"],
                true
            );
            assert_eq!(request(h, json!({"op":"unknown"}))["ok"], false);
            assert_eq!(decode(throne_macos_poll(u64::MAX, 0))["ok"], false);
            let huge = CString::new(vec![b'x'; INPUT_CAP + 1]).unwrap();
            assert_eq!(decode(throne_macos_request(h, huge.as_ptr()))["ok"], false);
            throne_macos_string_free(std::ptr::null_mut());
            assert_eq!(request(h, json!({"op":"select","id":id}))["ok"], true);
            assert_eq!(
                request(h, json!({"op":"snapshot"}))["value"]["settings"]["last_profile_id"],
                id
            );
            let mixed = request(
                h,
                json!({"op":"import_links","gid":gid,"text":"trojan://pw@b.example:443#b\ntrojan://broken"}),
            );
            assert_eq!(mixed["ok"], true, "{mixed}");
            assert_eq!(mixed["value"]["profiles"], 1);
            assert!(!mixed["value"]["errors"].as_array().unwrap().is_empty());
            assert_eq!(request(h, json!({"op":"check_config","id":id}))["ok"], true);
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                let snap = request(h, json!({"op":"snapshot"}));
                if snap["value"]["notice"].is_string() {
                    break;
                }
                assert!(
                    Instant::now() < until,
                    "missing core error not surfaced: {snap}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                request(h, json!({"op":"import_routes","path":"/missing"}))["ok"],
                false
            );
            assert_eq!(decode(throne_macos_close(h))["ok"], true);
            assert_eq!(request(h, json!({"op":"snapshot"}))["ok"], false);
            let reopened = decode(throne_macos_open(options.as_ptr()));
            let h = reopened["value"].as_u64().unwrap();
            tracing::info!("reopened-log-sink");
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                let snap = request(h, json!({"op":"snapshot"}));
                if snap["value"]["logs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v.as_str().unwrap().contains("reopened-log-sink"))
                {
                    break;
                }
                assert!(Instant::now() < until, "logs not routed after reopen");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                request(h, json!({"op":"snapshot"}))["value"]["settings"]["language"],
                "en"
            );
            assert_eq!(
                request(h, json!({"op":"group_delete","id":gid}))["ok"],
                true
            );
            assert_eq!(decode(throne_macos_close(h))["ok"], true);
            let mut s = State::new(dir.path().to_owned(), PathBuf::from("/missing")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(s.runtime.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert!(
                s.runtime
                    .path()
                    .join(format!("core-{}-{}.sock", std::process::id(), u64::MAX))
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    < 104
            );
            s.checking = true;
            assert!(s.mutable().is_err());
            s.checking = false;
            let mut p = throne_config::link::parse("trojan://pw@a.example:443#a").unwrap();
            p.gid = 1;
            let id = s.store.insert_profile(&p).unwrap();
            s.testing = true;
            s.latency.insert(id, fingerprint(&s.profile(id).unwrap()));
            s.reduce(Event::LatencyResult { id, latency: 42 }).unwrap();
            s.reduce(Event::TestFinished { tested: 1 }).unwrap();
            assert!(s.reduce(Event::LatencyResult { id, latency: 999 }).is_err());
            assert!(s.reduce(Event::TestFinished { tested: 1 }).is_err());
            s.reduce(Event::Status(Status::Connected {
                profile_id: id,
                name: "a".into(),
            }))
            .unwrap();
            s.reduce(Event::Traffic { down: 12, up: 3 }).unwrap();
            assert_eq!(s.store.profile(id).unwrap().unwrap().traffic_dl, 12);
            assert!(s.mutable().is_err());
            // Independent measurement slots remain available while connected.
            s.request(json!({"op":"test_latency","ids":[id]})).unwrap();
            s.request(json!({"op":"speed_test","id":id})).unwrap();
            s.request(json!({"op":"check_config","id":id})).unwrap();
            assert!(s.testing && s.speed_profile.is_some() && s.checking);
            assert!(s.request(json!({"op":"speed_test","id":id})).is_err());
            // Even an out-of-band Store edit cannot make an old result attach
            // to a reused/changed profile. Public mutation routes are guarded.
            let mut changed = s.profile(id).unwrap();
            changed.outbound["server"] = json!("replacement.invalid");
            s.store.update_profile(&changed).unwrap();
            assert!(s.reduce(Event::LatencyResult { id, latency: 999 }).is_err());
            assert!(s
                .reduce(Event::SpeedResult {
                    id,
                    download: "wrong".into(),
                    upload: String::new(),
                    country: String::new(),
                    latency: 999,
                    error: String::new()
                })
                .is_err());
            assert_eq!(s.profile(id).unwrap().latency, 42);
            assert!(s.profile(id).unwrap().dl_speed.is_empty());
            assert!(
                s.testing && s.checking && s.speed_profile.is_none(),
                "speed completion does not clear independent slots"
            );
            s.reduce(Event::TestFinished { tested: 0 }).unwrap();
            assert!(s.checking);
            assert!(s
                .reduce(Event::ConfigChecked(Err("fixture config failure".into())))
                .is_err());
            assert!(!s.checking);
            s.request(json!({"op":"stop_test"})).unwrap();
            s.request(json!({"op":"disconnect"})).unwrap();
            assert!(
                matches!(s.status, Status::Connected { .. }),
                "disconnect waits for the engine status event"
            );
            s.reduce_report(Event::Error("visible failure".into()));
            assert_eq!(s.notice.as_deref(), Some("visible failure"));
            for _ in 0..600 {
                s.reduce(Event::Log("log".into())).unwrap();
            }
            assert_eq!(s.logs.len(), 500);
            let (hold, controlled) = async_channel::unbounded();
            let actual = std::mem::replace(&mut s.events, controlled);
            // An unrelated open event sender must not prevent a real thread join.
            s.shutdown_timeout(Duration::from_secs(2)).unwrap();
            assert!(s.request(json!({"op":"connect","id":id})).is_err());
            drop(hold);
            s.shutdown_timeout(Duration::from_millis(10)).unwrap();
            s.events = actual;
            s.shutdown().unwrap();
        }
    }
}
