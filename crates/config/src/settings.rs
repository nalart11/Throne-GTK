//! Application settings. Everything affecting the generated configuration lives here,
//! is serialized into the `settings` table with one key per field and survives
//! the addition of new fields: unknown keys are ignored, and missing ones
//! come from `Default`.

use serde::{Deserialize, Serialize};

use crate::route::RouteRule;

/// How system traffic is intercepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// Local mixed port only: applications configure themselves.
    Proxy,
    /// TUN interface: all system traffic. Requires core privileges.
    Vpn,
}

impl ProxyMode {
    pub fn is_vpn(self) -> bool {
        matches!(self, ProxyMode::Vpn)
    }
}

/// TUN network stack implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunStack {
    System,
    GVisor,
    Mixed,
}

impl TunStack {
    pub fn as_str(self) -> &'static str {
        match self {
            TunStack::System => "system",
            TunStack::GVisor => "gvisor",
            TunStack::Mixed => "mixed",
        }
    }
}

/// Which addresses to resolve and where to connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainStrategy {
    AsIs,
    PreferIpv4,
    PreferIpv6,
    Ipv4Only,
    Ipv6Only,
}

impl DomainStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            DomainStrategy::AsIs => "",
            DomainStrategy::PreferIpv4 => "prefer_ipv4",
            DomainStrategy::PreferIpv6 => "prefer_ipv6",
            DomainStrategy::Ipv4Only => "ipv4_only",
            DomainStrategy::Ipv6Only => "ipv6_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ── inbound ──────────────────────────────────────────────────────────
    pub mode: ProxyMode,
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub sniffing: bool,

    // ── TUN ─────────────────────────────────────────────────────────────
    pub tun_stack: TunStack,
    pub tun_mtu: u32,
    pub tun_strict_route: bool,
    pub tun_ipv4_cidr: String,
    pub tun_ipv6_cidr: String,
    pub tun_ipv6: bool,

    // ── DNS ─────────────────────────────────────────────────────────────
    pub dns_enabled: bool,
    /// Resolver for proxied domains; connects through the proxy.
    pub dns_remote: String,
    /// Resolver for direct connections; connects directly. DoH is used by default
    /// instead of ordinary UDP: providers forge responses on port 53, preventing
    /// resolution outside the proxy, including the mirror from which the core
    /// downloads ready-made lists at startup.
    pub dns_direct: String,
    pub dns_strategy: DomainStrategy,
    pub dns_routing: bool,
    pub fakedns: bool,

    // ── routing ───────────────────────────────────────────────────────────
    pub bypass_private: bool,
    /// Rules defined in the interface. Order matters: the first matching rule applies.
    pub route_rules: Vec<RouteRule>,
    /// Additional route rules in their original sing-box form; inserted after
    /// interface rules and before automatic ones.
    pub custom_route_rules: String,

    // ── core ──────────────────────────────────────────────────────────────
    pub log_level: String,
    pub stats_enabled: bool,
    pub mux_enabled: bool,
    pub mux_protocol: String,
    pub mux_max_streams: u32,

    // ── tests ─────────────────────────────────────────────────────────────
    pub test_url: String,
    pub test_timeout_ms: i32,
    pub test_concurrency: i32,
    /// What to measure during a speed test. Upload is measured separately: few
    /// users need it, and it takes as long as download.
    pub speed_test_download: bool,
    pub speed_test_upload: bool,
    pub speed_test_timeout_ms: i32,

    // ── automatic server selection ────────────────────────────────────────
    /// How often the group's best servers are retested.
    pub auto_interval_secs: i64,
    /// How many milliseconds a candidate must beat the current server by for
    /// the connection to move. Without a margin, noise would make selection jump.
    pub auto_tolerance_ms: i32,
    /// Distribute connections across several good servers instead of one.
    pub auto_balance: bool,

    // ── subscriptions ─────────────────────────────────────────────────────
    pub sub_auto_update: bool,
    pub sub_auto_update_minutes: i64,
    pub sub_user_agent: String,
    pub sub_send_hwid: bool,
    pub sub_custom_hwid_params: String,

    /// Where the core stores its cache. Not saved to the database: this is an
    /// installation property, not a setting; the application sets the path at
    /// startup, otherwise the core writes `cache.db` to the current directory.
    #[serde(skip)]
    pub cache_file: String,

    // ── interface ─────────────────────────────────────────────────────────
    /// Closing the window hides the application in the tray instead of quitting it.
    /// Without a system tray icon, this setting has no effect: there is nowhere
    /// to hide the window.
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub connect_last_on_start: bool,
    pub last_profile_id: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: ProxyMode::Proxy,
            mixed_port: 2080,
            allow_lan: false,
            sniffing: true,

            tun_stack: TunStack::Mixed,
            tun_mtu: 1500,
            tun_strict_route: true,
            tun_ipv4_cidr: "172.19.0.1/30".into(),
            tun_ipv6_cidr: "fdfe:dcba:9876::1/126".into(),
            tun_ipv6: false,

            dns_enabled: true,
            dns_remote: "tls://1.1.1.1".into(),
            dns_direct: "https://1.1.1.1/dns-query".into(),
            dns_strategy: DomainStrategy::PreferIpv4,
            dns_routing: true,
            fakedns: false,

            bypass_private: true,
            route_rules: Vec::new(),
            custom_route_rules: String::new(),

            log_level: "info".into(),
            stats_enabled: true,
            mux_enabled: false,
            mux_protocol: "h2mux".into(),
            mux_max_streams: 8,

            test_url: "https://www.gstatic.com/generate_204".into(),
            test_timeout_ms: 5000,
            test_concurrency: 16,
            speed_test_download: true,
            speed_test_upload: false,
            speed_test_timeout_ms: 12000,

            auto_interval_secs: 180,
            auto_tolerance_ms: 150,
            auto_balance: false,

            sub_auto_update: false,
            sub_auto_update_minutes: 360,
            sub_user_agent: concat!("throne-gtk/", env!("CARGO_PKG_VERSION")).into(),
            sub_send_hwid: false,
            sub_custom_hwid_params: String::new(),

            cache_file: String::new(),

            close_to_tray: true,
            start_minimized: false,
            connect_last_on_start: false,
            last_profile_id: 0,
        }
    }
}
