//! Background executor: everything that cannot be done on the UI thread.
//!
//! The interface does not wait for the core or access the network itself. It sends
//! [`Command`] and receives [`Event`], so connecting, testing two hundred servers,
//! and updating a subscription do not freeze the window.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use nolurix_config::generate::GeneratedConfig;
use nolurix_config::{subscription, Profile, Settings};
use nolurix_ipc::pb;
use nolurix_ipc::Core;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task::JoinSet;

type Terminal = std::result::Result<(), String>;
const CLOSE_GRACE: Duration = Duration::from_secs(3);

/// A dedicated cancellation lane, independent of command/event backlogs.
#[derive(Clone)]
struct Cancel(watch::Receiver<bool>);
impl Cancel {
    fn requested(&self) -> bool {
        *self.0.borrow()
    }
    async fn wait(&self) {
        let mut receiver = self.0.clone();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
}
fn cancellation() -> (watch::Sender<bool>, Cancel) {
    let (tx, rx) = watch::channel(false);
    (tx, Cancel(rx))
}
fn cancelled() -> anyhow::Error {
    anyhow::anyhow!(crate::i18n::tr("операция отменена", "operation cancelled"))
}
async fn interrupt<T>(
    cancel: &Cancel,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! { biased; _=cancel.wait()=>Err(cancelled()), result=future=>result }
}

struct Lifecycle {
    thread: Option<std::thread::JoinHandle<()>>,
    done: std::sync::mpsc::Receiver<Terminal>,
    terminal: Option<Terminal>,
}
pub struct Engine {
    commands: mpsc::Sender<Command>,
    cancel: watch::Sender<bool>,
    lifecycle: Mutex<Lifecycle>,
    events: async_channel::Sender<Event>,
}
impl Engine {
    pub fn start(
        core_bin: PathBuf,
        runtime_dir: PathBuf,
        events: async_channel::Sender<Event>,
    ) -> Self {
        let output = events.clone();
        Self::spawn_worker(events, move |commands, cancel| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            let result = runtime.block_on(run(
                core_bin,
                runtime_dir,
                commands,
                output,
                cancel,
                Jobs::new(),
            ));
            // run joins every engine job; IPC close owns transport cleanup.
            drop(runtime);
            result
        })
    }
    fn spawn_worker(
        events: async_channel::Sender<Event>,
        work: impl FnOnce(mpsc::Receiver<Command>, Cancel) -> Result<()> + Send + 'static,
    ) -> Self {
        let (commands, receiver) = mpsc::channel(64);
        let (cancel, signal) = cancellation();
        let (done_tx, done) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("nolurix-engine".into())
            .spawn(move || {
                let terminal = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(receiver, signal)
                }))
                .map_err(|_| "engine worker panicked".to_string())
                .and_then(|result| result.map_err(|e| format!("{e:#}")));
                let _ = done_tx.send(terminal);
            })
            .expect("не удалось запустить фоновый поток");
        Self {
            commands,
            cancel,
            lifecycle: Mutex::new(Lifecycle {
                thread: Some(thread),
                done,
                terminal: None,
            }),
            events,
        }
    }
    pub fn request_shutdown(&self) {
        self.cancel.send_replace(true);
    }
    /// A timeout retains the real JoinHandle; later calls retry the same join.
    /// Cleanup failures are cached and never converted into successful shutdown.
    pub fn shutdown(&self, timeout: Duration) -> Result<()> {
        self.request_shutdown();
        let started = std::time::Instant::now();
        // Do not wait unboundedly behind another shutdown caller's mutex.
        let mut lifecycle = loop {
            match self.lifecycle.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    anyhow::bail!("engine lifecycle poisoned")
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    if started.elapsed() >= timeout {
                        anyhow::bail!("engine join timed out; retry shutdown");
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        if lifecycle.terminal.is_none() {
            let result = lifecycle
                .done
                .recv_timeout(timeout.saturating_sub(started.elapsed()))
                .context("engine join timed out; retry shutdown")?;
            let joined = lifecycle.thread.take().map(|thread| thread.join());
            lifecycle.terminal = Some(if matches!(joined, Some(Err(_))) {
                Err("engine thread panicked".into())
            } else {
                result
            });
        }
        lifecycle
            .terminal
            .as_ref()
            .unwrap()
            .clone()
            .map_err(anyhow::Error::msg)
    }
    pub fn send(&self, command: Command) {
        if let Err(error) = self.try_send(command) {
            let _ = self.events.try_send(Event::Error(format!("{error:#}")));
        }
    }
    pub fn try_send(&self, command: Command) -> Result<()> {
        if matches!(command, Command::Shutdown) {
            self.request_shutdown();
            return Ok(());
        }
        if *self.cancel.borrow() {
            anyhow::bail!(crate::i18n::tr(
                "исполнитель останавливается",
                "engine is shutting down"
            ));
        }
        self.commands
            .try_send(command)
            .map_err(|error| anyhow::anyhow!("engine command queue unavailable: {error}"))
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

#[derive(Clone)]
struct Output {
    events: async_channel::Sender<Event>,
    shutdown: Cancel,
}
impl Output {
    async fn emit(&self, event: Event) {
        // Terminal events are attempted once. Shutdown never waits for a UI to
        // drain its queue; if there is room it still receives the completion.
        if self.shutdown.requested() {
            let _ = self.events.try_send(event);
            return;
        }
        tokio::select! {biased; _=self.shutdown.wait()=>{}, _=self.events.send(event)=>{}}
    }
    async fn session(&self, cancel: &Cancel, event: Event) -> bool {
        tokio::select! {biased; _=cancel.wait()=>false, _=self.shutdown.wait()=>false, result=self.events.send(event)=>result.is_ok()}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Connection,
    Latency,
    Speed,
    Check,
    Subscription(i64),
}
struct Done {
    key: Key,
    error: Option<String>,
    cleanup_error: Option<String>,
}
struct SessionSlot {
    cancel: watch::Sender<bool>,
    controls: mpsc::Sender<AutoControl>,
}
enum AutoControl {
    Recheck,
    Pin(String),
}
struct Jobs {
    tasks: JoinSet<Done>,
    connection: Option<SessionSlot>,
    owned: HashMap<Key, watch::Sender<bool>>,
    pending: Option<Command>,
    cleanup_errors: Vec<String>,
}
impl Jobs {
    fn new() -> Self {
        Self {
            tasks: JoinSet::new(),
            connection: None,
            owned: HashMap::new(),
            pending: None,
            cleanup_errors: Vec::new(),
        }
    }
    fn cancel_all(&mut self) {
        self.pending = None;
        if let Some(session) = &self.connection {
            session.cancel.send_replace(true);
        }
        for cancel in self.owned.values() {
            cancel.send_replace(true);
        }
    }
    fn stop_tests(&self) {
        for key in [Key::Latency, Key::Speed] {
            if let Some(cancel) = self.owned.get(&key) {
                cancel.send_replace(true);
            }
        }
    }
    fn replace_connection(&mut self, command: Command) -> Option<Command> {
        if let Some(session) = &self.connection {
            session.cancel.send_replace(true);
            self.pending = Some(command);
            None
        } else {
            Some(command)
        }
    }
    fn spawn(
        &mut self,
        key: Key,
        job: impl FnOnce(Cancel) -> std::pin::Pin<Box<dyn std::future::Future<Output = Done> + Send>>,
    ) -> bool {
        if self.owned.contains_key(&key) {
            return false;
        }
        let (tx, cancel) = cancellation();
        self.owned.insert(key, tx);
        self.tasks.spawn(job(cancel));
        true
    }
}

async fn run(
    core_bin: PathBuf,
    runtime_dir: PathBuf,
    mut commands: mpsc::Receiver<Command>,
    events: async_channel::Sender<Event>,
    shutdown: Cancel,
    mut jobs: Jobs,
) -> Result<()> {
    let output = Output {
        events,
        shutdown: shutdown.clone(),
    };
    let mut stopping = false;
    loop {
        if stopping && jobs.tasks.is_empty() {
            break;
        }
        tokio::select! {biased;
            _=shutdown.wait(),if !stopping=>{stopping=true;commands.close();jobs.cancel_all();},
            result=jobs.tasks.join_next(),if !jobs.tasks.is_empty()=>{
                match result {
                    Some(Ok(done))=>{
                        if let Some(error)=done.cleanup_error {
                            if jobs.cleanup_errors.len()<16{jobs.cleanup_errors.push(error);}
                        }
                        if done.key==Key::Connection {
                            jobs.connection=None;
                            if !stopping {
                                if let Some(error)=done.error {output.emit(Event::Status(Status::Failed(error))).await;}
                                else if jobs.pending.is_none(){output.emit(Event::Status(Status::Disconnected)).await;}
                                if let Some(command)=jobs.pending.take(){
                                    if jobs.cleanup_errors.is_empty(){start_session(&mut jobs,command,&core_bin,&runtime_dir,&output).await;}
                                    else{output.emit(Event::Error(jobs.cleanup_errors.join("; "))).await;}
                                }
                            }
                        } else {jobs.owned.remove(&done.key);}
                    },
                    Some(Err(error))=>{
                        // Never claim a successful close after an unjoined/panicked job.
                        jobs.cleanup_errors.push(format!("engine job failed: {error}"));
                        jobs.cancel_all();stopping=true;commands.close();
                    },
                    None=>{},
                }
            },
            command=commands.recv(),if !stopping=>{
                let Some(command)=command else {stopping=true;jobs.cancel_all();continue;};
                match command {
                    Command::Shutdown=>{stopping=true;jobs.cancel_all();},
                    command @ (Command::Connect{..}|Command::ConnectAuto{..})=>{
                        if !jobs.cleanup_errors.is_empty(){output.emit(Event::Error(jobs.cleanup_errors.join("; "))).await;continue;}
                        if let Some(command)=jobs.replace_connection(command){start_session(&mut jobs,command,&core_bin,&runtime_dir,&output).await;}
                    },
                    Command::Disconnect=>{
                        jobs.pending=None;
                        if let Some(session)=&jobs.connection{session.cancel.send_replace(true);}
                        else{output.emit(Event::Status(Status::Disconnected)).await;}
                    },
                    Command::AutoPin(name)=>forward_auto(&jobs,AutoControl::Pin(name),&output).await,
                    Command::AutoRecheck=>forward_auto(&jobs,AutoControl::Recheck,&output).await,
                    Command::StopTest=>jobs.stop_tests(),
                    command=>{
                        let key=match &command{Command::TestLatency{..}=>Key::Latency,Command::SpeedTest{..}=>Key::Speed,Command::CheckConfig{..}=>Key::Check,Command::FetchSubscription{gid,..}=>Key::Subscription(*gid),_=>unreachable!()};
                        if jobs.owned.contains_key(&key) || matches!(key,Key::Subscription(_)) && jobs.owned.keys().filter(|key|matches!(key,Key::Subscription(_))).count()>=4 {
                            output.emit(Event::Error(crate::i18n::tr("операция уже выполняется или очередь занята","operation already running or queue busy").into())).await;continue;
                        }
                        let bin=core_bin.clone();let dir=runtime_dir.clone();let out=output.clone();
                        jobs.spawn(key,move |cancel|Box::pin(owned_job(command,bin,dir,out,cancel)));
                    },
                }
            }
        }
    }
    if jobs.cleanup_errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", jobs.cleanup_errors.join("; "))
    }
}
async fn forward_auto(jobs: &Jobs, control: AutoControl, output: &Output) {
    let result = jobs
        .connection
        .as_ref()
        .map(|session| session.controls.try_send(control));
    if !matches!(result, Some(Ok(()))) {
        output
            .emit(Event::Error(
                crate::i18n::tr(
                    "автовыбор недоступен или очередь занята",
                    "auto selection unavailable or queue busy",
                )
                .into(),
            ))
            .await;
    }
}
async fn start_session(jobs: &mut Jobs, command: Command, bin: &Path, dir: &Path, output: &Output) {
    let (tx, cancel) = cancellation();
    let (controls, rx) = mpsc::channel(8);
    jobs.connection = Some(SessionSlot {
        cancel: tx,
        controls,
    });
    output.emit(Event::Status(Status::Connecting)).await;
    let bin = bin.to_owned();
    let dir = dir.to_owned();
    let output = output.clone();
    jobs.tasks
        .spawn(async move { connection_job(command, bin, dir, output, cancel, rx).await });
}

struct SessionConfig {
    generated: GeneratedConfig,
    settings: Box<Settings>,
    id: i64,
    name: String,
    tags: Vec<String>,
    members: Vec<(String, String)>,
}
fn session_config(command: Command) -> Result<SessionConfig> {
    let (generated, settings, id, name, members) = match command {
        Command::Connect { profile, settings } => {
            let generated = nolurix_config::generate(&profile, &settings).context(
                crate::i18n::tr("сборка конфига", "generating configuration"),
            )?;
            (generated, settings, profile.id, profile.name, Vec::new())
        }
        Command::ConnectAuto {
            profiles,
            gid,
            settings,
        } => {
            if profiles.is_empty() {
                anyhow::bail!(crate::i18n::tr(
                    "в группе нет серверов",
                    "the group has no servers"
                ));
            }
            let generated = nolurix_config::generate::generate_auto(&profiles, &settings).context(
                crate::i18n::tr("сборка конфига", "generating configuration"),
            )?;
            let members = generated
                .tags
                .iter()
                .cloned()
                .zip(profiles.iter().map(|p| p.name.clone()))
                .collect();
            let name = crate::tr_format!(
                "Автовыбор из {}",
                "Automatic selection from {}",
                profiles.len()
            );
            (generated, settings, -gid, name, members)
        }
        _ => unreachable!(),
    };
    let mut tags = generated.tags.clone();
    if !members.is_empty() {
        tags.insert(0, nolurix_config::generate::tags::PROXY.into());
    }
    Ok(SessionConfig {
        generated,
        settings,
        id,
        name,
        tags,
        members,
    })
}
async fn close_core<T>(core: &mut Core, result: Result<T>) -> (Result<T>, Option<String>) {
    match core.close(CLOSE_GRACE).await {
        Ok(()) => (result, None),
        Err(error) => {
            let cleanup = format!("core cleanup unconfirmed: {error:#}");
            let combined = match result {
                Ok(_) => cleanup.clone(),
                Err(error) => format!("{error:#}; {cleanup}"),
            };
            (Err(anyhow::anyhow!(combined)), Some(cleanup))
        }
    }
}
async fn connection_job(
    command: Command,
    bin: PathBuf,
    dir: PathBuf,
    output: Output,
    cancel: Cancel,
    controls: mpsc::Receiver<AutoControl>,
) -> Done {
    let mut cleanup_error = None;
    let mut safe_cancel = false;
    let result = async {
        let config = session_config(command)?;
        if cancel.requested() {
            safe_cancel = true;
            return Ok(());
        }
        // Do not drop a privileged StartCore handshake on cancellation. IPC owns
        // its bound; once it returns, cancellation closes the obtained Core.
        let mut core = if config.settings.mode.is_vpn() {
            match Core::spawn_privileged(&bin, &dir, config.settings.log_level == "debug").await {
                Ok(core) => core,
                Err(error) => {
                    cleanup_error = Some(format!(
                        "privileged spawn failed; root cleanup unconfirmed: {error:#}"
                    ));
                    return Err(error);
                }
            }
        } else {
            Core::spawn(&bin, &dir, config.settings.log_level == "debug").await?
        };
        let result = session_body(&mut core, &config, &bin, &output, &cancel, controls).await;
        let (result, cleanup) = close_core(&mut core, result).await;
        safe_cancel = cleanup.is_none();
        cleanup_error = cleanup;
        result
    }
    .await;
    let error = if cancel.requested() && safe_cancel {
        None
    } else {
        result.err().map(|e| format!("{e:#}"))
    };
    Done {
        key: Key::Connection,
        error,
        cleanup_error,
    }
}
async fn session_body(
    core: &mut Core,
    config: &SessionConfig,
    bin: &Path,
    output: &Output,
    cancel: &Cancel,
    mut controls: mpsc::Receiver<AutoControl>,
) -> Result<()> {
    if cancel.requested() {
        return Ok(());
    }
    if config.settings.mode.is_vpn() && !interrupt(cancel, core.client.is_privileged()).await? {
        anyhow::bail!(vpn_privilege_hint(bin));
    }
    // A cancellation observed during spawn/privilege checks never submits Start.
    interrupt(
        cancel,
        core.client
            .start(load_request(&config.generated, &config.settings)),
    )
    .await
    .context(crate::i18n::tr(
        "запуск конфига в ядре",
        "starting configuration in core",
    ))?;
    if !output
        .session(
            cancel,
            Event::Status(Status::Connected {
                profile_id: config.id,
                name: config.name.clone(),
            }),
        )
        .await
    {
        return Ok(());
    }
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {biased;
            _=cancel.wait()=>return Ok(()),
            control=controls.recv()=>{
                if let Some(control)=control {
                    let(action,member)=match control {
                        AutoControl::Recheck=>("recheck",None),
                        AutoControl::Pin(name)=>("select",Some(config.members.iter().find(|(_,n)|*n==name).map(|(tag,_)|tag.clone()).unwrap_or_default())),
                    };
                    interrupt(cancel,core.client.auto_selector_action(pb::AutoSelectorActionRequest{tag:Some(nolurix_config::generate::tags::PROXY.into()),action:Some(action.into()),member})).await?;
                }
            },
            _=poll.tick()=>{
                if let Some(status)=core.exited(){anyhow::bail!(crate::tr_format!("ядро завершилось ({status})","core exited ({status})"));}
                let stats=interrupt(cancel,core.client.query_stats()).await?;
                let sum=|map:&std::collections::HashMap<String,i64>|config.tags.iter().filter_map(|tag|map.get(tag)).sum();
                if !output.session(cancel,Event::Traffic{down:sum(&stats.downs),up:sum(&stats.ups)}).await{return Ok(());}
                if !config.members.is_empty(){
                    let response=interrupt(cancel,core.client.query_auto_selectors()).await?;
                    if let Some(group)=response.groups.first(){
                        let name=|tag:&str|config.members.iter().find(|(t,_)|t==tag).map(|(_,n)|n.clone()).unwrap_or_default();
                        if !output.session(cancel,Event::AutoStatus{selected:name(group.selected()),pinned:name(group.pinned()),alive:group.members_alive(),total:group.members_total(),suspended:group.suspended()}).await{return Ok(());}
                    }
                }
                let conns=interrupt(cancel,core.client.query_connections()).await?;
                let list=conns.active.into_iter().map(|c|Connection{id:c.id().into(),dest:c.dest().into(),domain:c.domain().into(),network:c.network().into(),process:c.process().into(),upload:c.upload(),download:c.download(),created_at:c.created_at()}).collect();
                if !output.session(cancel,Event::Connections(list)).await{return Ok(());}
            }
        }
    }
}

async fn owned_job(
    command: Command,
    bin: PathBuf,
    dir: PathBuf,
    output: Output,
    cancel: Cancel,
) -> Done {
    let mut cleanup_error = None;
    let key = match command {
        Command::TestLatency {
            profiles,
            ids,
            settings,
        } => {
            let result = latency_job(
                &bin,
                &dir,
                profiles,
                ids,
                &settings,
                &output,
                &cancel,
                &mut cleanup_error,
            )
            .await;
            let tested = match result {
                Ok(tested) => tested,
                Err(error) => {
                    output.emit(Event::Error(format!("{error:#}"))).await;
                    0
                }
            };
            output.emit(Event::TestFinished { tested }).await;
            Key::Latency
        }
        Command::SpeedTest {
            profile,
            id,
            settings,
        } => {
            let result = speed_job(
                &bin,
                &dir,
                *profile,
                id,
                &settings,
                &output,
                &cancel,
                &mut cleanup_error,
            )
            .await;
            let event = result.unwrap_or_else(|error| Event::SpeedResult {
                id,
                download: String::new(),
                upload: String::new(),
                country: String::new(),
                latency: 0,
                error: format!("{error:#}"),
            });
            output.emit(event).await;
            Key::Speed
        }
        Command::CheckConfig { profile, settings } => {
            let result = check_job(&bin, &dir, &profile, &settings, &cancel, &mut cleanup_error)
                .await
                .map_err(|e| format!("{e:#}"));
            output.emit(Event::ConfigChecked(result)).await;
            Key::Check
        }
        Command::FetchSubscription {
            gid,
            url,
            user_agent,
            send_hwid,
            custom_hwid_params,
        } => {
            let result = interrupt(
                &cancel,
                fetch_subscription(&url, &user_agent, send_hwid, &custom_hwid_params),
            )
            .await
            .map_err(|e| format!("{e:#}"));
            output.emit(Event::Subscription { gid, result }).await;
            Key::Subscription(gid)
        }
        _ => unreachable!(),
    };
    Done {
        key,
        error: None,
        cleanup_error,
    }
}
async fn latency_job(
    bin: &Path,
    dir: &Path,
    profiles: Vec<Profile>,
    ids: Vec<i64>,
    settings: &Settings,
    output: &Output,
    cancel: &Cancel,
    cleanup: &mut Option<String>,
) -> Result<usize> {
    if profiles.is_empty() {
        return Ok(0);
    }
    let generated = nolurix_config::generate_test(&profiles, settings).context(crate::i18n::tr(
        "сборка конфига теста",
        "generating test configuration",
    ))?;
    if cancel.requested() {
        return Err(cancelled());
    }
    let mut core = Core::spawn(bin, dir, false).await?;
    let result = async {
        let request = pb::TestReq {
            config: Some(generated.core_config.clone()),
            outbound_tags: generated.tags.clone(),
            url: Some(settings.test_url.clone()),
            max_concurrency: Some(settings.test_concurrency),
            test_timeout_ms: Some(settings.test_timeout_ms),
            need_xray: Some(generated.need_xray),
            xray_config: Some(generated.xray_config.clone()),
            ..Default::default()
        };
        let response = test_rpc(cancel, &core.client, core.client.test(request)).await?;
        let mut tested = 0;
        for result in response.results {
            if cancel.requested() {
                return Err(cancelled());
            }
            let Some(index) = generated
                .tags
                .iter()
                .position(|tag| tag == result.outbound_tag())
            else {
                continue;
            };
            let Some(&id) = ids.get(index) else { continue };
            let latency = if result.error().is_empty() {
                result.latency_ms().max(1)
            } else {
                -1
            };
            if !output
                .session(cancel, Event::LatencyResult { id, latency })
                .await
            {
                return Err(cancelled());
            }
            tested += 1;
        }
        Ok(tested)
    }
    .await;
    let (result, error) = close_core(&mut core, result).await;
    *cleanup = error;
    result
}
/// StopTest always uses the measurement's own client, never a connection client.
async fn test_rpc<T>(
    cancel: &Cancel,
    client: &nolurix_ipc::CoreClient,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    measurement_rpc(cancel, future, client.stop_test()).await
}
async fn measurement_rpc<T>(
    cancel: &Cancel,
    future: impl std::future::Future<Output = Result<T>>,
    stop: impl std::future::Future<Output = Result<()>>,
) -> Result<T> {
    // Biased cancellation means the RPC is not even polled/submitted when a
    // cancellation was received during the bounded spawn handshake.
    tokio::select! {biased;
        _=cancel.wait()=>{stop.await.context("stopping owned measurement")?;Err(cancelled())},
        result=future=>result,
    }
}
async fn check_job(
    bin: &Path,
    dir: &Path,
    profile: &Profile,
    settings: &Settings,
    cancel: &Cancel,
    cleanup: &mut Option<String>,
) -> Result<()> {
    let generated = nolurix_config::generate(profile, settings).context(crate::i18n::tr(
        "сборка конфига",
        "generating configuration",
    ))?;
    if cancel.requested() {
        return Err(cancelled());
    }
    let mut core = Core::spawn(bin, dir, false).await?;
    let result = interrupt(
        cancel,
        core.client.check_config(load_request(&generated, settings)),
    )
    .await;
    let (result, error) = close_core(&mut core, result).await;
    *cleanup = error;
    result
}
async fn speed_job(
    bin: &Path,
    dir: &Path,
    profile: Profile,
    id: i64,
    settings: &Settings,
    output: &Output,
    cancel: &Cancel,
    cleanup: &mut Option<String>,
) -> Result<Event> {
    let generated =
        nolurix_config::generate_test(&[profile], settings).context(crate::i18n::tr(
            "сборка конфига замера",
            "generating speed test configuration",
        ))?;
    if cancel.requested() {
        return Err(cancelled());
    }
    let mut core = Core::spawn(bin, dir, false).await?;
    let result=async {
        let mut request=nolurix_ipc::speed_test_request(generated.core_config.clone());
        request.outbound_tags=generated.tags.clone();request.test_download=Some(settings.speed_test_download);request.test_upload=Some(settings.speed_test_upload);request.timeout_ms=Some(settings.speed_test_timeout_ms);request.need_xray=Some(generated.need_xray);request.xray_config=Some(generated.xray_config.clone());
        let pending=core.client.speed_test(request);tokio::pin!(pending);
        let mut tick=tokio::time::interval(Duration::from_millis(700));tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let response=loop {
            tokio::select! {biased;
                _=cancel.wait()=>return Err(cancelled()),
                result=&mut pending=>break result?,
                _=tick.tick()=>{
                    let state=interrupt(cancel,core.client.query_speed_test()).await?;
                    if state.is_running(){if let Some(result)=state.result{
                        let stage=if !result.dl_speed().is_empty(){crate::tr_format!("приём {}","download {}",result.dl_speed())}else if !result.server_name().is_empty(){crate::tr_format!("сервер {}","server {}",result.server_name())}else{crate::i18n::tr("замеряю…","measuring…").to_string()};
                        if !output.session(cancel,Event::SpeedProgress{id,stage}).await{return Err(cancelled());}
                    }}
                }
            }
        };
        let result=response.results.into_iter().next().ok_or_else(||anyhow::anyhow!(crate::i18n::tr("ядро не вернуло результат замера","core returned no speed test result")))?;
        Ok(Event::SpeedResult{id,download:result.dl_speed().into(),upload:result.ul_speed().into(),country:result.server_country().into(),latency:result.latency(),error:result.error().into()})
    }.await;
    // Cancellation during a progress query follows the same owned StopTest
    // path as cancellation while awaiting the measurement response.
    let result = if cancel.requested() {
        match core.client.stop_test().await {
            Ok(()) => Err(cancelled()),
            Err(error) => Err(error.context("stopping owned measurement")),
        }
    } else {
        result
    };
    let (result, error) = close_core(&mut core, result).await;
    *cleanup = error;
    result
}

/// What the interface requests.
#[derive(Debug)]
pub enum Command {
    Connect {
        profile: Box<Profile>,
        settings: Box<Settings>,
    },
    /// Connect with auto-selection: the core keeps the best server in the group.
    ConnectAuto {
        profiles: Vec<Profile>,
        /// Group associated with auto-selection, needed so the interface can
        /// return the marker to the correct row.
        gid: i64,
        settings: Box<Settings>,
    },
    /// Recheck all auto-selection members right now.
    AutoRecheck,
    /// Pin a member manually; an empty string removes the pin.
    AutoPin(String),
    Disconnect,
    /// Test latency across the profile list. `ids` runs alongside `profiles` and
    /// is returned in events so the interface knows which row to update.
    TestLatency {
        profiles: Vec<Profile>,
        ids: Vec<i64>,
        settings: Box<Settings>,
    },
    StopTest,
    /// Measure one server’s speed. It runs in its own core and does not affect
    /// the active connection, just like latency testing.
    SpeedTest {
        profile: Box<Profile>,
        id: i64,
        settings: Box<Settings>,
    },
    FetchSubscription {
        gid: i64,
        url: String,
        user_agent: String,
        send_hwid: bool,
        custom_hwid_params: String,
    },
    /// Check that the configuration builds and is accepted by the core.
    CheckConfig {
        profile: Box<Profile>,
        settings: Box<Settings>,
    },
    Shutdown,
}

/// Connection state shown by the main screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected { profile_id: i64, name: String },
    Failed(String),
}

/// What happened in the background.
#[derive(Debug)]
pub enum Event {
    Status(Status),
    /// Bytes during the last interval; the interface calculates the speed.
    Traffic {
        down: i64,
        up: i64,
    },
    Connections(Vec<Connection>),
    /// Auto-selection state: who is selected and how many servers are alive.
    AutoStatus {
        selected: String,
        pinned: String,
        alive: i32,
        total: i32,
        suspended: bool,
    },
    LatencyResult {
        id: i64,
        latency: i32,
    },
    TestFinished {
        tested: usize,
    },
    /// Intermediate measurement state: the core returns it on request while the
    /// test runs, otherwise it is unclear for a minute whether the test is alive.
    SpeedProgress {
        id: i64,
        stage: String,
    },
    SpeedResult {
        id: i64,
        download: String,
        upload: String,
        country: String,
        latency: i32,
        error: String,
    },
    Subscription {
        gid: i64,
        result: Result<subscription::Parsed, String>,
    },
    ConfigChecked(Result<(), String>),
    Log(String),
    Error(String),
}

/// A live connection in a table-friendly form.
#[derive(Debug, Clone)]
pub struct Connection {
    pub id: String,
    pub dest: String,
    pub domain: String,
    pub network: String,
    pub process: String,
    pub upload: i64,
    pub download: i64,
    pub created_at: i64,
}

fn load_request(generated: &GeneratedConfig, settings: &Settings) -> pb::LoadConfigReq {
    let mut request = nolurix_ipc::load_request(generated.core_config.clone());
    request.disable_stats = Some(!settings.stats_enabled);
    request.need_xray = Some(generated.need_xray);
    request.xray_config = Some(generated.xray_config.clone());
    request.tun_ipv4_cidr = Some(generated.tun_ipv4_cidr.clone());
    request
}

fn vpn_privilege_hint(_core_bin: &Path) -> String {
    #[cfg(target_os = "linux")]
    return format!(
        "{} {}",
        crate::i18n::tr("для режима VPN ядру нужны права на создание TUN-интерфейса.\nВыдайте их один раз: sudo setcap cap_net_admin,cap_net_raw+ep", "VPN mode requires permission to create a TUN interface.\nGrant it once: sudo setcap cap_net_admin,cap_net_raw+ep"),
        _core_bin.display()
    );

    #[cfg(target_os = "windows")]
    return crate::i18n::tr(
        "для режима VPN перезапустите Nolurix от имени администратора",
        "restart Nolurix as administrator for VPN mode",
    )
    .into();

    #[cfg(target_os = "macos")]
    return crate::i18n::tr(
        "root-helper запущен, но не получил права на создание TUN-интерфейса",
        "root-helper started but lacks permission to create a TUN interface",
    )
    .into();

    #[allow(unreachable_code)]
    crate::i18n::tr(
        "для режима VPN нужны права на создание TUN-интерфейса",
        "VPN mode requires permission to create a TUN interface",
    )
    .into()
}

async fn fetch_subscription(
    url: &str,
    user_agent: &str,
    send_hwid: bool,
    custom_hwid_params: &str,
) -> Result<subscription::Parsed> {
    let (body, userinfo) =
        subscription::fetch(url, user_agent, 30, send_hwid, custom_hwid_params).await?;
    let mut parsed = subscription::parse(&body)?;
    if !userinfo.is_empty() {
        parsed.info =
            subscription::format_userinfo_for_language(&userinfo, crate::i18n::language());
    }
    Ok(parsed)
}

#[cfg(test)]
mod supervision_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Instant;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn done(key: Key) -> Done {
        Done {
            key,
            error: None,
            cleanup_error: None,
        }
    }
    fn empty_auto(gid: i64) -> Command {
        Command::ConnectAuto {
            profiles: Vec::new(),
            gid,
            settings: Box::default(),
        }
    }
    #[test]
    fn idle_restart_and_full_event_queue_shutdown_join() {
        for _ in 0..2 {
            let (tx, rx) = async_channel::bounded(1);
            tx.try_send(Event::Log("queue occupied".into())).unwrap();
            let engine = Engine::start(
                PathBuf::from("/never-used-core"),
                PathBuf::from("/never-used-runtime"),
                tx,
            );
            // Coordinator may be waiting to emit Disconnected. Cancellation
            // bypasses both its full output and the bounded command backlog.
            for _ in 0..70 {
                engine.send(Command::Disconnect);
            }
            engine.send(Command::Shutdown);
            engine.shutdown(Duration::from_secs(1)).unwrap();
            engine.shutdown(Duration::ZERO).unwrap();
            let lifecycle = engine.lifecycle.lock().unwrap();
            assert!(lifecycle.thread.is_none());
            assert!(matches!(lifecycle.terminal, Some(Ok(()))));
            assert_eq!(rx.len(), 1);
        }
    }
    #[test]
    fn shutdown_timeout_release_join_retry_and_cached_failure() {
        let (tx, _rx) = async_channel::bounded(1);
        let (release, wait) = std::sync::mpsc::channel();
        let (ready, started) = std::sync::mpsc::channel();
        let engine = Engine::spawn_worker(tx, move |_commands, cancel| {
            ready.send(()).unwrap();
            wait.recv().unwrap();
            assert!(cancel.requested());
            Ok(())
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        // A synthetic handshake cannot yet complete, and the command receiver
        // deliberately does not drain. Shutdown still requests cancellation now.
        for _ in 0..64 {
            engine.send(Command::Disconnect);
        }
        let before = Instant::now();
        engine.request_shutdown();
        engine.request_shutdown();
        assert!(before.elapsed() < Duration::from_millis(100));
        assert!(engine.shutdown(Duration::from_millis(2)).is_err());
        assert!(engine.lifecycle.lock().unwrap().thread.is_some());
        release.send(()).unwrap();
        engine.shutdown(Duration::from_secs(1)).unwrap();
        assert!(engine.lifecycle.lock().unwrap().thread.is_none());
        let (tx, _rx) = async_channel::bounded(1);
        let failed =
            Engine::spawn_worker(tx, |_, _| anyhow::bail!("controlled cleanup unconfirmed"));
        let error = failed
            .shutdown(Duration::from_secs(1))
            .unwrap_err()
            .to_string();
        assert!(error.contains("cleanup unconfirmed"));
        assert_eq!(
            failed.shutdown(Duration::ZERO).unwrap_err().to_string(),
            error
        );
        assert!(failed.lifecycle.lock().unwrap().thread.is_none());
    }
    #[test]
    fn drop_requests_shutdown_without_waiting() {
        let (tx, _rx) = async_channel::bounded(1);
        let (ack, observed) = std::sync::mpsc::channel();
        let engine = Engine::spawn_worker(tx, move |_, cancel| {
            runtime().block_on(cancel.wait());
            ack.send(()).unwrap();
            Ok(())
        });
        let before = Instant::now();
        drop(engine);
        assert!(before.elapsed() < Duration::from_millis(100));
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn full_event_queue_cannot_hold_an_owned_job_during_shutdown() {
        let (events, receiver) = async_channel::bounded(1);
        events.try_send(Event::Log("do not drain".into())).unwrap();
        let output_events = events.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let engine = Engine::spawn_worker(events, move |commands, shutdown| {
            runtime().block_on(async move {
                let mut jobs = Jobs::new();
                let output = Output {
                    events: output_events.clone(),
                    shutdown: shutdown.clone(),
                };
                jobs.spawn(Key::Latency, move |_| {
                    Box::pin(async move {
                        ready.send(()).unwrap();
                        output.emit(Event::TestFinished { tested: 0 }).await;
                        done(Key::Latency)
                    })
                });
                run(
                    PathBuf::new(),
                    PathBuf::new(),
                    commands,
                    output_events,
                    shutdown,
                    jobs,
                )
                .await
            })
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        engine.shutdown(Duration::from_secs(1)).unwrap();
        assert!(engine.lifecycle.lock().unwrap().thread.is_none());
        assert_eq!(receiver.len(), 1);
    }
    #[test]
    fn stop_test_cancels_both_owned_measurements_not_connection_or_check() {
        let (events, receiver) = async_channel::bounded(16);
        let output_events = events.clone();
        let session_stopped = Arc::new(AtomicBool::new(false));
        let session_flag = session_stopped.clone();
        let check_stopped = Arc::new(AtomicBool::new(false));
        let check_flag = check_stopped.clone();
        let completions = Arc::new(AtomicUsize::new(0));
        let job_completions = completions.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let engine = Engine::spawn_worker(events, move |commands, shutdown| {
            runtime().block_on(async move {
                let mut jobs = Jobs::new();
                let output = Output {
                    events: output_events.clone(),
                    shutdown: shutdown.clone(),
                };
                let (cancel, signal) = cancellation();
                let (controls, _rx) = mpsc::channel(1);
                jobs.connection = Some(SessionSlot { cancel, controls });
                jobs.tasks.spawn(async move {
                    signal.wait().await;
                    session_flag.store(true, Ordering::SeqCst);
                    done(Key::Connection)
                });
                for key in [Key::Latency, Key::Speed] {
                    let output = output.clone();
                    let count = job_completions.clone();
                    assert!(jobs.spawn(key, move |cancel| Box::pin(async move {
                        cancel.wait().await;
                        count.fetch_add(1, Ordering::SeqCst);
                        output
                            .emit(match key {
                                Key::Latency => Event::TestFinished { tested: 0 },
                                _ => Event::SpeedResult {
                                    id: 99,
                                    download: String::new(),
                                    upload: String::new(),
                                    country: String::new(),
                                    latency: 0,
                                    error: "cancelled".into(),
                                },
                            })
                            .await;
                        done(key)
                    })));
                }
                jobs.spawn(Key::Check, move |cancel| {
                    Box::pin(async move {
                        cancel.wait().await;
                        check_flag.store(true, Ordering::SeqCst);
                        done(Key::Check)
                    })
                });
                ready.send(()).unwrap();
                run(
                    PathBuf::new(),
                    PathBuf::new(),
                    commands,
                    output_events,
                    shutdown,
                    jobs,
                )
                .await
            })
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        engine.send(Command::StopTest);
        let mut latency = 0;
        let mut speed = 0;
        let until = Instant::now() + Duration::from_secs(1);
        while latency + speed < 2 {
            if let Ok(event) = receiver.try_recv() {
                match event {
                    Event::TestFinished { .. } => latency += 1,
                    Event::SpeedResult { id, .. } => {
                        assert_eq!(id, 99);
                        speed += 1;
                    }
                    _ => panic!("unexpected event"),
                }
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!((latency, speed), (1, 1));
        assert_eq!(completions.load(Ordering::SeqCst), 2);
        assert!(!session_stopped.load(Ordering::SeqCst));
        assert!(!check_stopped.load(Ordering::SeqCst));
        engine.send(Command::Disconnect);
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            if let Ok(Event::Status(Status::Disconnected)) = receiver.try_recv() {
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(session_stopped.load(Ordering::SeqCst));
        assert!(!check_stopped.load(Ordering::SeqCst));
        engine.shutdown(Duration::from_secs(1)).unwrap();
        assert!(check_stopped.load(Ordering::SeqCst));
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn cancellation_before_measurement_submission_and_owned_stop_routing() {
        runtime().block_on(async {
            let(tx,cancel)=cancellation();tx.send_replace(true);
            let submitted=AtomicUsize::new(0);let own_stops=AtomicUsize::new(0);let connection_stops=AtomicUsize::new(0);
            let result=measurement_rpc(&cancel,async {submitted.fetch_add(1,Ordering::SeqCst);Ok(())},async {own_stops.fetch_add(1,Ordering::SeqCst);Ok(())}).await;
            assert!(result.is_err());assert_eq!(submitted.load(Ordering::SeqCst),0);
            assert_eq!(own_stops.load(Ordering::SeqCst),1);assert_eq!(connection_stops.load(Ordering::SeqCst),0);
            let(tx,cancel)=cancellation();
            let pending=measurement_rpc(&cancel,async {submitted.fetch_add(1,Ordering::SeqCst);std::future::pending::<Result<()>>().await},async {own_stops.fetch_add(1,Ordering::SeqCst);Ok(())});
            tokio::pin!(pending);
            tokio::select!{_=&mut pending=>panic!("hung job completed"),_=tokio::time::sleep(Duration::from_millis(1))=>{}}
            tx.send_replace(true);assert!(pending.await.is_err());
            assert_eq!(submitted.load(Ordering::SeqCst),1);assert_eq!(own_stops.load(Ordering::SeqCst),2);
            assert_eq!(connection_stops.load(Ordering::SeqCst),0);
        });
    }
    #[test]
    fn cancelled_jobs_emit_exactly_one_completion_without_spawning() {
        runtime().block_on(async {
            let (_root, shutdown) = cancellation();
            let (tx, events) = async_channel::bounded(16);
            let output = Output {
                events: tx,
                shutdown,
            };
            let (tx, cancel) = cancellation();
            tx.send_replace(true);
            let profile =
                nolurix_config::link::parse("trojan://pw@no-network.invalid:443#fixture").unwrap();
            let settings = Box::<Settings>::default();
            let dir = std::env::temp_dir().join(format!(
                "engine-never-created-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let commands = vec![
                Command::TestLatency {
                    profiles: vec![profile.clone()],
                    ids: vec![7],
                    settings: settings.clone(),
                },
                Command::SpeedTest {
                    profile: Box::new(profile.clone()),
                    id: 7,
                    settings: settings.clone(),
                },
                Command::CheckConfig {
                    profile: Box::new(profile),
                    settings,
                },
                Command::FetchSubscription {
                    gid: 4,
                    url: "https://no-network.invalid/".into(),
                    user_agent: String::new(),
                    send_hwid: false,
                    custom_hwid_params: String::new(),
                },
            ];
            for command in commands {
                let done = owned_job(
                    command,
                    PathBuf::from("/never-run"),
                    dir.clone(),
                    output.clone(),
                    cancel.clone(),
                )
                .await;
                assert!(done.cleanup_error.is_none());
            }
            assert!(!dir.exists(), "cancelled handshake must not reach spawn");
            let (mut latency, mut errors, mut speed, mut check, mut sub) = (0, 0, 0, 0, 0);
            while let Ok(event) = events.try_recv() {
                match event {
                    Event::Error(_) => errors += 1,
                    Event::TestFinished { .. } => latency += 1,
                    Event::SpeedResult { error, .. } => {
                        assert!(!error.is_empty());
                        speed += 1;
                    }
                    Event::ConfigChecked(result) => {
                        assert!(result.is_err());
                        check += 1;
                    }
                    Event::Subscription { result, .. } => {
                        assert!(result.is_err());
                        sub += 1;
                    }
                    _ => panic!("unexpected nonterminal event"),
                }
            }
            assert_eq!((latency, errors, speed, check, sub), (1, 1, 1, 1, 1));
        });
    }
    #[test]
    fn reconnect_keeps_one_pending_replacement_until_old_cleanup() {
        runtime().block_on(async {
            let mut jobs = Jobs::new();
            let (tx, cancel) = cancellation();
            let (controls, _) = mpsc::channel(1);
            jobs.connection = Some(SessionSlot {
                cancel: tx,
                controls,
            });
            assert!(jobs.replace_connection(empty_auto(1)).is_none());
            assert!(jobs.replace_connection(empty_auto(2)).is_none());
            assert!(cancel.requested());
            assert!(matches!(
                jobs.pending,
                Some(Command::ConnectAuto { gid: 2, .. })
            ));
            assert!(
                jobs.tasks.is_empty(),
                "no second session before old cleanup"
            );
            jobs.cancel_all();
            assert!(jobs.pending.is_none());
        });
        let (events, rx) = async_channel::bounded(8);
        let output = events.clone();
        let (release, gate) = tokio::sync::oneshot::channel();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let (ready, started) = std::sync::mpsc::channel();
        let engine = Engine::spawn_worker(events, move |commands, shutdown| {
            runtime().block_on(async move {
                let mut jobs = Jobs::new();
                let (tx, cancel) = cancellation();
                let (controls, _) = mpsc::channel(1);
                jobs.connection = Some(SessionSlot {
                    cancel: tx,
                    controls,
                });
                jobs.tasks.spawn(async move {
                    cancel.wait().await;
                    cancelled_tx.send(()).unwrap();
                    gate.await.unwrap();
                    done(Key::Connection)
                });
                ready.send(()).unwrap();
                run(
                    PathBuf::from("/never-run"),
                    PathBuf::new(),
                    commands,
                    output,
                    shutdown,
                    jobs,
                )
                .await
            })
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        engine.send(empty_auto(2));
        cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(rx.try_recv().is_err());
        release.send(()).unwrap();
        // Empty group fails generation before any spawn, but only AFTER the old
        // session's simulated close has completed and its job was joined.
        let until = Instant::now() + Duration::from_secs(1);
        let mut statuses = Vec::new();
        while statuses.len() < 2 {
            if let Ok(Event::Status(status)) = rx.try_recv() {
                statuses.push(status);
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(statuses[0], Status::Connecting);
        assert!(matches!(&statuses[1], Status::Failed(_)));
        engine.shutdown(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn background_cancellation_joins_and_drops_network_work() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        runtime().block_on(async {
            let flag = Arc::new(AtomicBool::new(false));
            let guard = Dropped(flag.clone());
            let mut jobs = Jobs::new();
            jobs.spawn(Key::Subscription(1), move |cancel| {
                Box::pin(async move {
                    let _guard = guard;
                    cancel.wait().await;
                    done(Key::Subscription(1))
                })
            });
            jobs.cancel_all();
            jobs.tasks.join_next().await.unwrap().unwrap();
            assert!(jobs.tasks.is_empty());
            assert!(flag.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn cancellation_awaits_workers_and_completes_each_type_once() {
        // The owned-job replacement must preserve the legacy completion contract.
        cancelled_jobs_emit_exactly_one_completion_without_spawning();
        stop_test_cancels_both_owned_measurements_not_connection_or_check();
    }

    #[test]
    fn cancelled_generation_cannot_forward_queued_results() {
        runtime().block_on(async {
            let (_root, shutdown) = cancellation();
            let (events, receiver) = async_channel::bounded(2);
            let output = Output { events, shutdown };
            let (tx, cancel) = cancellation();
            tx.send_replace(true);
            assert!(
                !output
                    .session(&cancel, Event::LatencyResult { id: 7, latency: 42 })
                    .await
            );
            assert!(receiver.try_recv().is_err());
        });
    }
    #[test]
    fn coordinator_cleanup_failure_is_joined_and_never_success() {
        let (events, _rx) = async_channel::bounded(1);
        let output = events.clone();
        let engine = Engine::spawn_worker(events, move |commands, shutdown| {
            runtime().block_on(async move {
                let mut jobs = Jobs::new();
                jobs.spawn(Key::Check, |cancel| {
                    Box::pin(async move {
                        cancel.wait().await;
                        Done {
                            key: Key::Check,
                            error: None,
                            cleanup_error: Some("synthetic Core::close failure".into()),
                        }
                    })
                });
                run(
                    PathBuf::new(),
                    PathBuf::new(),
                    commands,
                    output,
                    shutdown,
                    jobs,
                )
                .await
            })
        });
        let error = engine
            .shutdown(Duration::from_secs(1))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Core::close failure"));
        assert!(engine.lifecycle.lock().unwrap().thread.is_none());
        assert_eq!(
            engine.shutdown(Duration::ZERO).unwrap_err().to_string(),
            error
        );
    }
}
