//! Настройки приложения. Всё, что влияет на генерируемый конфиг, живёт здесь,
//! сериализуется в таблицу `settings` по одному ключу на поле и переживает
//! добавление новых полей: неизвестные ключи игнорируются, отсутствующие
//! берутся из `Default`.

use serde::{Deserialize, Serialize};

use crate::route::RouteRule;

/// Как перехватывается трафик системы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// Только локальный mixed-порт: приложения настраиваются сами.
    Proxy,
    /// TUN-интерфейс: весь трафик системы. Требует привилегий у ядра.
    Vpn,
}

impl ProxyMode {
    pub fn is_vpn(self) -> bool {
        matches!(self, ProxyMode::Vpn)
    }
}

/// Реализация сетевого стека TUN.
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

/// Какие адреса резолвить и куда ходить.
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
    // ── вход ────────────────────────────────────────────────────────────
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
    /// Резолвер для проксируемых доменов; ходит через прокси.
    pub dns_remote: String,
    /// Резолвер для прямых соединений; ходит напрямую. По умолчанию DoH, а не
    /// обычный UDP: провайдеры подделывают ответы на 53-м порту, и тогда мимо
    /// прокси не резолвится ничего — включая зеркало, с которого ядро тянет
    /// готовые списки при старте.
    pub dns_direct: String,
    pub dns_strategy: DomainStrategy,
    pub dns_routing: bool,
    pub fakedns: bool,

    // ── маршрутизация ───────────────────────────────────────────────────
    pub bypass_private: bool,
    /// Правила, заданные в интерфейсе. Порядок важен: применяется первое
    /// подошедшее.
    pub route_rules: Vec<RouteRule>,
    /// Дополнительные правила route в исходном виде sing-box; вставляются
    /// после правил из интерфейса, перед автоматическими.
    pub custom_route_rules: String,

    // ── ядро ────────────────────────────────────────────────────────────
    pub log_level: String,
    pub stats_enabled: bool,
    pub mux_enabled: bool,
    pub mux_protocol: String,
    pub mux_max_streams: u32,

    // ── тесты ───────────────────────────────────────────────────────────
    pub test_url: String,
    pub test_timeout_ms: i32,
    pub test_concurrency: i32,
    /// Что мерить при замере скорости. Отдача считается отдельно: она нужна
    /// далеко не всем, а времени занимает столько же, сколько приём.
    pub speed_test_download: bool,
    pub speed_test_upload: bool,
    pub speed_test_timeout_ms: i32,

    // ── автовыбор сервера ───────────────────────────────────────────────
    /// Как часто перепроверяются лучшие серверы группы.
    pub auto_interval_secs: i64,
    /// На сколько миллисекунд претендент должен обгонять текущий сервер,
    /// чтобы соединение переехало. Без запаса выбор дёргался бы от шума.
    pub auto_tolerance_ms: i32,
    /// Раскладывать соединения по нескольким хорошим серверам вместо одного.
    pub auto_balance: bool,

    // ── подписки ────────────────────────────────────────────────────────
    pub sub_auto_update: bool,
    pub sub_auto_update_minutes: i64,
    pub sub_user_agent: String,
    pub sub_send_hwid: bool,
    pub sub_custom_hwid_params: String,

    /// Куда ядро складывает свой кэш. Не сохраняется в базу: это свойство
    /// установки, а не настройка — приложение проставляет путь при запуске,
    /// иначе ядро пишет `cache.db` в текущий каталог.
    #[serde(skip)]
    pub cache_file: String,

    // ── интерфейс ───────────────────────────────────────────────────────
    /// Закрытие окна прячет программу в значок, а не выключает её. Без
    /// значка в системе настройка ни на что не влияет: спрятать окно
    /// некуда.
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
            tun_mtu: 9000,
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
