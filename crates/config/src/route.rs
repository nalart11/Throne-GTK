//! Правила маршрутизации.
//!
//! Правило отвечает на один вопрос: что делать с соединением, подходящим под
//! условие. Набор условий намеренно ограничен теми, которыми пользуются:
//! домены, адреса, порты, процессы и готовые списки. Всё остальное задаётся
//! сырым JSON — см. [`Settings::custom_route_rules`](crate::Settings).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// По какому признаку правило узнаёт соединение.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Домен и все его поддомены: `.example.com`.
    DomainSuffix,
    /// Домен целиком, без поддоменов.
    Domain,
    /// Подстрока в домене.
    DomainKeyword,
    /// Регулярное выражение по домену.
    DomainRegex,
    /// Адрес или подсеть: `10.0.0.0/8`.
    IpCidr,
    /// Порт назначения.
    Port,
    /// Имя процесса: `Telegram`, `firefox`.
    Process,
    /// Готовый список sing-box: `geosite-ru`, `geoip-de`.
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

    /// Название для интерфейса.
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

    /// Подсказка о том, что вписывать.
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

/// Что сделать с подошедшим соединением.
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

/// Одно условие правила: признак и значения, по которым узнаётся соединение.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Condition {
    pub kind: MatchKind,
    /// Пустой список означает условие-заглушку — в конфиг оно не попадает:
    /// без значений условие подошло бы всему подряд.
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

    /// Условие в формате sing-box без действия. `None` — значений нет.
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
        // Порты — числа, всё остальное строки. Диапазон вида `8000:8100`
        // sing-box ждёт в отдельном поле.
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

    /// Строка для списка условий в редакторе правила.
    pub fn summary(&self) -> String {
        let values: Vec<&String> = self.values.iter().filter(|v| !v.trim().is_empty()).collect();
        match values.len() {
            0 => "условие не задано".into(),
            1 => values[0].clone(),
            n => format!("{} и ещё {}", values[0], n - 1),
        }
    }
}

/// Как связаны условия одного правила.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Достаточно одного совпадения — правило-список исключений.
    #[default]
    Any,
    /// Должны совпасть все: «этот процесс и только на этом порту».
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

    /// Разделитель для перечисления условий в списке правил.
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
    /// Условия правила. Пустой список означает правило-заглушку — такое
    /// в конфиг не попадает: без условий оно перехватило бы весь трафик.
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

/// Правило в том виде, в каком его писали версии с одним условием на правило.
/// Настройки живут в базе пользователя и переживают обновление программы, так
/// что старую запись надо уметь прочесть.
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
    /// Правило с одним условием — обычный случай.
    pub fn single(kind: MatchKind, values: Vec<String>, action: RouteAction) -> Self {
        Self {
            conditions: vec![Condition::new(kind, values)],
            action,
            ..Default::default()
        }
    }

    /// Правило в формате sing-box. `None` — правило выключено или пусто.
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
            // Несколько признаков в одном правиле sing-box связывает по своим
            // правилам: домены с подсетями — через «или», всё остальное — через
            // «и». Чтобы правило работало так, как выбрано в интерфейсе, а не
            // как совпало, условия заворачиваются в logical с явным режимом.
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

    /// Готовые списки надо объявить в `route.rule_set`, иначе конфиг не
    /// соберётся. Возвращает имена списков, которые использует правило.
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

    /// Строка для списка правил в настройках.
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

    /// Условия одной строкой: «Домен и поддомены или Процесс».
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

/// Зеркало, через которое качаются готовые списки.
///
/// Сами списки лежат на GitHub, но обращаться к нему напрямую нельзя
/// рассчитывать: там, где он заблокирован, соединение не поднимется вовсе —
/// ядро не стартует, пока не соберёт все `rule_set`. jsDelivr отдаёт те же
/// файлы из репозитория и доступен там, где GitHub нет. Так же поступает и
/// оригинальный Throne.
const MIRROR: &str = "https://testingcf.jsdelivr.net/gh";

/// Объявление готового списка для `route.rule_set`.
///
/// Списки берутся из репозиториев sing-geosite и sing-geoip: имя вида
/// `geosite-ru` или `geoip-ru` однозначно задаёт адрес.
pub fn rule_set_declaration(name: &str) -> Option<Value> {
    let (repo, file) = if let Some(rest) = name.strip_prefix("geosite-") {
        ("sing-geosite", format!("geosite-{rest}"))
    } else if let Some(rest) = name.strip_prefix("geoip-") {
        ("sing-geoip", format!("geoip-{rest}"))
    } else {
        return None;
    };

    Some(json!({
        "type": "remote",
        "tag": name,
        "format": "binary",
        // Ветка `rule-set` в репозитории — у jsDelivr она задаётся через `@`.
        "url": format!("{MIRROR}/SagerNet/{repo}@rule-set/{file}.srs"),
        // Списки качаются напрямую: если тянуть их через прокси, первое
        // подключение упирается в само себя.
        "download_detour": "direct",
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

    /// Домены и подсети sing-box и так связывает через «или», а процессы и
    /// готовые списки — через «и». Режим правила должен решать за него.
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
        // Действие принадлежит правилу целиком: ядро отвергает конфиг, если
        // действие указано ещё и внутри подправила.
        assert_eq!(v["action"], "route");
        assert_eq!(v["outbound"], "direct");
        assert!(v["rules"][0].get("action").is_none());

        let strict = RouteRule {
            match_mode: MatchMode::All,
            ..rule
        };
        assert_eq!(strict.to_singbox().unwrap()["mode"], "and");
    }

    /// Пустые условия не должны превращать правило в logical с одной веткой.
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
        assert!(v.get("type").is_none(), "осталось одно условие — правило плоское");
        assert_eq!(v["domain"][0], "mos.ru");
    }

    #[test]
    fn disabled_and_empty_rules_are_skipped() {
        let disabled = RouteRule {
            enabled: false,
            ..RouteRule::single(MatchKind::Domain, vec!["a.example".into()], RouteAction::Direct)
        };
        assert!(disabled.to_singbox().is_none());

        let empty = RouteRule::single(MatchKind::Domain, vec!["  ".into()], RouteAction::Direct);
        assert!(empty.to_singbox().is_none(), "правило без условий перехватило бы всё");

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

        let site = rule_set_declaration("geosite-ru").unwrap();
        assert_eq!(site["tag"], "geosite-ru");
        assert!(site["url"].as_str().unwrap().contains("sing-geosite"));
        assert!(
            site["url"].as_str().unwrap().starts_with(MIRROR),
            "списки качаются через зеркало: напрямую GitHub доступен не везде"
        );
        let ip = rule_set_declaration("geoip-ru").unwrap();
        assert!(ip["url"].as_str().unwrap().contains("sing-geoip"));
        assert!(rule_set_declaration("мой-список").is_none());
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
        assert_eq!(pair.display_name(), "Домен целиком или Процесс: mos.ru и ещё 1");
    }

    /// Настройки переживают обновление программы: правило, записанное версией
    /// с одним условием, должно читаться как правило с одним условием.
    #[test]
    fn rules_from_older_versions_are_read() {
        let old: RouteRule = serde_json::from_str(
            r#"{"name":"Default","enabled":true,"kind":"domain_suffix",
                "values":["example.com"],"action":"direct"}"#,
        )
        .unwrap();
        assert_eq!(
            old.conditions,
            vec![Condition::new(MatchKind::DomainSuffix, vec!["example.com".into()])]
        );
        assert_eq!(old.match_mode, MatchMode::Any);
        assert_eq!(old.action, RouteAction::Direct);
        assert_eq!(old.to_singbox().unwrap()["domain_suffix"][0], "example.com");

        // Запись нового формата читается как есть, старые поля не мешают.
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
