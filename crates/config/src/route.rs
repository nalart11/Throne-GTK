//! Routing rules.
//!
//! A rule answers one question: what to do with a connection matching a
//! condition. The set of conditions is deliberately limited to those in common use:
//! domains, addresses, ports, processes, and ready-made lists. Everything else is specified
//! as raw JSON; see [`Settings::custom_route_rules`](crate::Settings).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The criterion by which a rule identifies a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Domain and all its subdomains: `.example.com`.
    DomainSuffix,
    /// Entire domain, without subdomains.
    Domain,
    /// Substring in the domain.
    DomainKeyword,
    /// Regular expression for the domain.
    DomainRegex,
    /// Address or subnet: `10.0.0.0/8`.
    IpCidr,
    /// Destination port.
    Port,
    /// Process name: `Telegram`, `firefox`.
    Process,
    /// Ready-made sing-box list: `geosite-ru`, `geoip-de`.
    RuleSet,
}

impl MatchKind {
    pub fn all() -> &'static [MatchKind] {
        use MatchKind::*;
        &[
            DomainSuffix,
            Domain,
            DomainKeyword,
            DomainRegex,
            IpCidr,
            Port,
            Process,
            RuleSet,
        ]
    }

    /// Name for the interface.
    pub fn title(self) -> &'static str {
        match self {
            MatchKind::DomainSuffix => "Домен и поддомены",
            MatchKind::Domain => "Домен целиком",
            MatchKind::DomainKeyword => "Часть домена",
            MatchKind::DomainRegex => "Домен по выражению",
            MatchKind::IpCidr => "Адрес или подсеть",
            MatchKind::Port => "Порт",
            MatchKind::Process => "Процесс",
            MatchKind::RuleSet => "Готовый список",
        }
    }

    /// Hint about what to enter.
    pub fn hint(self) -> &'static str {
        match self {
            MatchKind::DomainSuffix => "example.com — сам домен и все поддомены",
            MatchKind::Domain => "example.com — только он",
            MatchKind::DomainKeyword => "часть имени, например: googlevideo",
            MatchKind::DomainRegex => "регулярное выражение: ^ads\\..*",
            MatchKind::IpCidr => "10.0.0.0/8 или 192.168.1.5",
            MatchKind::Port => "443 или диапазон 8000:8100",
            MatchKind::Process => "firefox, Telegram",
            MatchKind::RuleSet => "geosite-ru, geoip-ru — скачиваются автоматически",
        }
    }

    fn field(self) -> &'static str {
        match self {
            MatchKind::DomainSuffix => "domain_suffix",
            MatchKind::Domain => "domain",
            MatchKind::DomainKeyword => "domain_keyword",
            MatchKind::DomainRegex => "domain_regex",
            MatchKind::IpCidr => "ip_cidr",
            MatchKind::Port => "port",
            MatchKind::Process => "process_name",
            MatchKind::RuleSet => "rule_set",
        }
    }
}

/// What to do with a matching connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteAction {
    Proxy,
    Direct,
    Block,
}

impl RouteAction {
    pub fn all() -> &'static [RouteAction] {
        &[RouteAction::Proxy, RouteAction::Direct, RouteAction::Block]
    }

    pub fn title(self) -> &'static str {
        match self {
            RouteAction::Proxy => "Через прокси",
            RouteAction::Direct => "Напрямую",
            RouteAction::Block => "Блокировать",
        }
    }
}

/// One rule condition: the criterion and values used to identify a connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Condition {
    pub kind: MatchKind,
    /// An empty list denotes a placeholder condition and is omitted from the configuration:
    /// without values, it would match everything.
    pub values: Vec<String>,
}

impl Default for Condition {
    fn default() -> Self {
        Self {
            kind: MatchKind::DomainSuffix,
            values: Vec::new(),
        }
    }
}

impl Condition {
    pub fn new(kind: MatchKind, values: Vec<String>) -> Self {
        Self { kind, values }
    }

    /// Condition in sing-box format without an action. `None` means no values.
    fn to_singbox(&self) -> Option<Map<String, Value>> {
        let values: Vec<String> = self
            .values
            .iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        if values.is_empty() {
            return None;
        }

        let mut item = Map::new();
        // Ports are numbers, everything else is a string. A range such as `8000:8100`
        // is expected by sing-box in a separate field.
        if self.kind == MatchKind::Port {
            let mut ports = Vec::new();
            let mut ranges = Vec::new();
            for value in &values {
                match value.parse::<u16>() {
                    Ok(port) => ports.push(json!(port)),
                    Err(_) => ranges.push(json!(value)),
                }
            }
            if !ports.is_empty() {
                item.insert("port".into(), Value::Array(ports));
            }
            if !ranges.is_empty() {
                item.insert("port_range".into(), Value::Array(ranges));
            }
        } else {
            item.insert(self.kind.field().into(), json!(values));
        }
        Some(item)
    }

    /// String for the condition list in the rule editor.
    pub fn summary(&self) -> String {
        let values: Vec<&String> = self
            .values
            .iter()
            .filter(|v| !v.trim().is_empty())
            .collect();
        match values.len() {
            0 => "условие не задано".into(),
            1 => values[0].clone(),
            n => format!("{} и ещё {}", values[0], n - 1),
        }
    }
}

/// How the conditions of one rule are connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// One match is enough: an exception-list rule.
    #[default]
    Any,
    /// All must match: “this process and only on this port”.
    All,
}

impl MatchMode {
    pub fn all() -> &'static [MatchMode] {
        &[MatchMode::Any, MatchMode::All]
    }

    pub fn title(self) -> &'static str {
        match self {
            MatchMode::Any => "Подошло любое условие",
            MatchMode::All => "Подошли все условия",
        }
    }

    /// Separator for listing conditions in the rule list.
    fn separator(self) -> &'static str {
        match self {
            MatchMode::Any => " или ",
            MatchMode::All => " и ",
        }
    }

    fn singbox(self) -> &'static str {
        match self {
            MatchMode::Any => "or",
            MatchMode::All => "and",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteRule {
    pub name: String,
    pub enabled: bool,
    /// Rule conditions. An empty list denotes a placeholder rule and is omitted
    /// from the configuration: without conditions it would intercept all traffic.
    pub conditions: Vec<Condition>,
    pub match_mode: MatchMode,
    pub action: RouteAction,
}

impl Default for RouteRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            conditions: Vec::new(),
            match_mode: MatchMode::Any,
            action: RouteAction::Direct,
        }
    }
}

/// A rule in the format written by versions with one condition per rule.
/// Settings live in the user's database and survive program updates, so
/// old records must remain readable.
#[derive(Deserialize)]
#[serde(default)]
struct StoredRule {
    name: String,
    enabled: bool,
    conditions: Vec<Condition>,
    match_mode: MatchMode,
    action: RouteAction,
    kind: Option<MatchKind>,
    values: Option<Vec<String>>,
}

impl Default for StoredRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            conditions: Vec::new(),
            match_mode: MatchMode::Any,
            action: RouteAction::Direct,
            kind: None,
            values: None,
        }
    }
}

impl<'de> Deserialize<'de> for RouteRule {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredRule::deserialize(deserializer)?;
        let mut conditions = stored.conditions;
        if conditions.is_empty() {
            if let (Some(kind), Some(values)) = (stored.kind, stored.values) {
                conditions.push(Condition { kind, values });
            }
        }
        Ok(RouteRule {
            name: stored.name,
            enabled: stored.enabled,
            conditions,
            match_mode: stored.match_mode,
            action: stored.action,
        })
    }
}

impl RouteRule {
    /// A rule with one condition, the usual case.
    pub fn single(kind: MatchKind, values: Vec<String>, action: RouteAction) -> Self {
        Self {
            conditions: vec![Condition::new(kind, values)],
            action,
            ..Default::default()
        }
    }

    /// Rule in sing-box format. `None` means the rule is disabled or empty.
    pub fn to_singbox(&self) -> Option<Value> {
        if !self.enabled {
            return None;
        }
        let parts: Vec<Map<String, Value>> = self
            .conditions
            .iter()
            .filter_map(|condition| condition.to_singbox())
            .collect();

        let mut rule = match parts.len() {
            0 => return None,
            1 => parts.into_iter().next().unwrap_or_default(),
            // sing-box combines several criteria in one rule according to its own
            // rules: domains with subnets use “or”, while everything else uses
            // “and”. To make the rule work as selected in the interface rather than
            // according to incidental matching, conditions are wrapped in logical with an explicit mode.
            _ => {
                let mut logical = Map::new();
                logical.insert("type".into(), json!("logical"));
                logical.insert("mode".into(), json!(self.match_mode.singbox()));
                logical.insert(
                    "rules".into(),
                    Value::Array(parts.into_iter().map(Value::Object).collect()),
                );
                logical
            }
        };

        match self.action {
            RouteAction::Block => {
                rule.insert("action".into(), json!("reject"));
            }
            RouteAction::Direct => {
                rule.insert("action".into(), json!("route"));
                rule.insert("outbound".into(), json!("direct"));
            }
            RouteAction::Proxy => {
                rule.insert("action".into(), json!("route"));
                rule.insert("outbound".into(), json!("proxy"));
            }
        }
        Some(Value::Object(rule))
    }

    /// Ready-made lists must be declared in `route.rule_set`, otherwise the configuration
    /// cannot be built. Returns the names of lists used by the rule.
    pub fn rule_sets(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        self.conditions
            .iter()
            .filter(|condition| condition.kind == MatchKind::RuleSet)
            .flat_map(|condition| condition.values.iter())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect()
    }

    /// String for the rule list in settings.
    pub fn summary(&self) -> String {
        let values: Vec<&String> = self
            .conditions
            .iter()
            .flat_map(|condition| condition.values.iter())
            .filter(|v| !v.trim().is_empty())
            .collect();
        match values.len() {
            0 => "условие не задано".into(),
            1 => values[0].clone(),
            n => format!("{} и ещё {}", values[0], n - 1),
        }
    }

    /// Conditions on one line: “Domain and subdomains or Process”.
    pub fn conditions_title(&self) -> String {
        if self.conditions.is_empty() {
            return "Условий нет".into();
        }
        self.conditions
            .iter()
            .map(|condition| condition.kind.title())
            .collect::<Vec<_>>()
            .join(self.match_mode.separator())
    }

    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.clone();
        }
        format!("{}: {}", self.conditions_title(), self.summary())
    }
}

/// Mirror used to download ready-made lists.
///
/// The lists themselves are hosted on GitHub, but direct access cannot be
/// relied upon: where it is blocked, the connection will not come up at all;
/// the core will not start until it loads all `rule_set`s. jsDelivr serves the same
/// repository files and is available where GitHub is not. The original
/// Throne does the same.
const MIRROR: &str = "https://testingcf.jsdelivr.net/gh";

/// Declaration of a ready-made list for `route.rule_set`.
///
/// Lists come from the sing-geosite and sing-geoip repositories: a name such as
/// `geosite-ru` or `geoip-ru` unambiguously determines the address.
pub fn rule_set_declaration(name: &str, download_detour: &str) -> Option<Value> {
    let (repo, file) = if let Some(rest) = name.strip_prefix("geosite-") {
        ("sing-geosite", format!("geosite-{rest}"))
    } else {
        let rest = name.strip_prefix("geoip-")?;
        ("sing-geoip", format!("geoip-{rest}"))
    };

    Some(json!({
        "type": "remote",
        "tag": name,
        "format": "binary",
        // The repository's `rule-set` branch is specified with `@` on jsDelivr.
        "url": format!("{MIRROR}/SagerNet/{repo}@rule-set/{file}.srs"),
        // The working configuration downloads lists through the proxy: direct jsDelivr access is
        // unreliable because of blocks, and a failed download brings down the entire connection.
        // The proxy is absent in tests, so they download directly.
        "download_detour": download_detour,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_rule_to_singbox() {
        let rule = RouteRule {
            name: "Российские сайты".into(),
            ..RouteRule::single(
                MatchKind::DomainSuffix,
                vec!["ru".into(), "рф".into()],
                RouteAction::Direct,
            )
        };
        let v = rule.to_singbox().unwrap();
        assert_eq!(v["domain_suffix"][0], "ru");
        assert_eq!(v["action"], "route");
        assert_eq!(v["outbound"], "direct");
    }

    #[test]
    fn block_action_uses_reject() {
        let rule = RouteRule::single(
            MatchKind::DomainKeyword,
            vec!["ads".into()],
            RouteAction::Block,
        );
        let v = rule.to_singbox().unwrap();
        assert_eq!(v["action"], "reject");
        assert!(v.get("outbound").is_none());
    }

    #[test]
    fn ports_split_into_numbers_and_ranges() {
        let rule = RouteRule::single(
            MatchKind::Port,
            vec!["443".into(), "8000:8100".into()],
            RouteAction::Proxy,
        );
        let v = rule.to_singbox().unwrap();
        assert_eq!(v["port"][0], 443);
        assert_eq!(v["port_range"][0], "8000:8100");
    }

    /// sing-box already combines domains and subnets with “or”, and processes
    /// and ready-made lists with “and”. The rule mode must decide for it.
    #[test]
    fn several_conditions_become_a_logical_rule() {
        let rule = RouteRule {
            conditions: vec![
                Condition::new(MatchKind::Domain, vec!["mos.ru".into()]),
                Condition::new(MatchKind::IpCidr, vec!["10.0.0.0/8".into()]),
                Condition::new(MatchKind::Process, vec!["cs2".into()]),
            ],
            action: RouteAction::Direct,
            ..Default::default()
        };

        let v = rule.to_singbox().unwrap();
        assert_eq!(v["type"], "logical");
        assert_eq!(v["mode"], "or", "по умолчанию хватает одного условия");
        assert_eq!(v["rules"].as_array().unwrap().len(), 3);
        assert_eq!(v["rules"][0]["domain"][0], "mos.ru");
        assert_eq!(v["rules"][2]["process_name"][0], "cs2");
        // The action belongs to the rule as a whole: the core rejects the
        // configuration if the action is also specified inside a subrule.
        assert_eq!(v["action"], "route");
        assert_eq!(v["outbound"], "direct");
        assert!(v["rules"][0].get("action").is_none());

        let strict = RouteRule {
            match_mode: MatchMode::All,
            ..rule
        };
        assert_eq!(strict.to_singbox().unwrap()["mode"], "and");
    }

    /// Empty conditions must not turn a rule into a logical rule with one branch.
    #[test]
    fn empty_conditions_do_not_count() {
        let rule = RouteRule {
            conditions: vec![
                Condition::new(MatchKind::Domain, vec!["mos.ru".into()]),
                Condition::new(MatchKind::Process, vec!["  ".into()]),
            ],
            ..Default::default()
        };
        let v = rule.to_singbox().unwrap();
        assert!(
            v.get("type").is_none(),
            "осталось одно условие — правило плоское"
        );
        assert_eq!(v["domain"][0], "mos.ru");
    }

    #[test]
    fn disabled_and_empty_rules_are_skipped() {
        let disabled = RouteRule {
            enabled: false,
            ..RouteRule::single(
                MatchKind::Domain,
                vec!["a.example".into()],
                RouteAction::Direct,
            )
        };
        assert!(disabled.to_singbox().is_none());

        let empty = RouteRule::single(MatchKind::Domain, vec!["  ".into()], RouteAction::Direct);
        assert!(
            empty.to_singbox().is_none(),
            "правило без условий перехватило бы всё"
        );

        let nothing = RouteRule::default();
        assert!(nothing.to_singbox().is_none());
    }

    #[test]
    fn rule_set_names_become_declarations() {
        let rule = RouteRule {
            conditions: vec![
                Condition::new(MatchKind::Domain, vec!["mos.ru".into()]),
                Condition::new(
                    MatchKind::RuleSet,
                    vec!["geosite-ru".into(), "geoip-ru".into()],
                ),
            ],
            action: RouteAction::Direct,
            ..Default::default()
        };
        assert_eq!(rule.rule_sets(), vec!["geosite-ru", "geoip-ru"]);

        let site = rule_set_declaration("geosite-ru", "direct").unwrap();
        assert_eq!(site["tag"], "geosite-ru");
        assert!(site["url"].as_str().unwrap().contains("sing-geosite"));
        assert!(
            site["url"].as_str().unwrap().starts_with(MIRROR),
            "списки качаются через зеркало: напрямую GitHub доступен не везде"
        );
        let ip = rule_set_declaration("geoip-ru", "proxy").unwrap();
        assert!(ip["url"].as_str().unwrap().contains("sing-geoip"));
        assert_eq!(ip["download_detour"], "proxy");
        assert!(rule_set_declaration("мой-список", "direct").is_none());
    }

    #[test]
    fn display_name_falls_back_to_condition() {
        let rule = RouteRule::single(
            MatchKind::Process,
            vec!["firefox".into()],
            RouteAction::Direct,
        );
        assert_eq!(rule.display_name(), "Процесс: firefox");

        let pair = RouteRule {
            conditions: vec![
                Condition::new(MatchKind::Domain, vec!["mos.ru".into()]),
                Condition::new(MatchKind::Process, vec!["cs2".into()]),
            ],
            ..Default::default()
        };
        assert_eq!(pair.conditions_title(), "Домен целиком или Процесс");
        assert_eq!(
            pair.display_name(),
            "Домен целиком или Процесс: mos.ru и ещё 1"
        );
    }

    /// Settings survive program updates: a rule written by a version with one
    /// condition must be read as a rule with one condition.
    #[test]
    fn rules_from_older_versions_are_read() {
        let old: RouteRule = serde_json::from_str(
            r#"{"name":"Default","enabled":true,"kind":"domain_suffix",
                "values":["example.com"],"action":"direct"}"#,
        )
        .unwrap();
        assert_eq!(
            old.conditions,
            vec![Condition::new(
                MatchKind::DomainSuffix,
                vec!["example.com".into()]
            )]
        );
        assert_eq!(old.match_mode, MatchMode::Any);
        assert_eq!(old.action, RouteAction::Direct);
        assert_eq!(old.to_singbox().unwrap()["domain_suffix"][0], "example.com");

        // A record in the new format is read as is; old fields do not interfere.
        let new: RouteRule = serde_json::from_str(
            r#"{"name":"","enabled":true,"match_mode":"all",
                "conditions":[{"kind":"process","values":["cs2"]},
                              {"kind":"port","values":["443"]}],
                "action":"proxy"}"#,
        )
        .unwrap();
        assert_eq!(new.conditions.len(), 2);
        assert_eq!(new.match_mode, MatchMode::All);
    }
}
