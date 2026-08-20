//! Модель профиля. `outbound` хранится ровно в том виде, в каком его ждёт
//! sing-box, — это и формат хранения, и то, что уходит в ядро. Никакого
//! промежуточного представления протоколов: любой новый протокол sing-box
//! начинает работать без изменений в этом крейте.

use anyhow::{bail, Result};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: i64,
    /// Тип outbound-а sing-box: vless, vmess, shadowsocks, trojan, hysteria2, …
    /// Плюс наши собственные: `xrayvless` и `custom`.
    pub kind: String,
    pub name: String,
    pub gid: i64,
    /// Последняя задержка в мс. 0 — не тестировался, <0 — тест провалился.
    pub latency: i32,
    pub dl_speed: String,
    pub ul_speed: String,
    pub test_country: String,
    pub ip_out: String,
    pub outbound: Value,
    pub traffic_dl: i64,
    pub traffic_up: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub latency_at: i64,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: 0,
            kind: "vless".into(),
            name: String::new(),
            gid: 0,
            latency: 0,
            dl_speed: String::new(),
            ul_speed: String::new(),
            test_country: String::new(),
            ip_out: String::new(),
            outbound: Value::Object(Map::new()),
            traffic_dl: 0,
            traffic_up: 0,
            created_at: 0,
            updated_at: 0,
            latency_at: 0,
        }
    }
}

impl Profile {
    pub fn from_outbound(outbound: Value) -> Result<Self> {
        let Some(obj) = outbound.as_object() else {
            bail!("outbound должен быть JSON-объектом");
        };
        // Два родных формата: sing-box (`type`) и Xray (`protocol` +
        // `streamSettings`). Xray-форма приходит из баз Throne и от панелей,
        // раздающих готовый конфиг; переводить её в sing-box нельзя без потерь —
        // xmux, downloadSettings и прочие расширения там не выражаются.
        let kind = match obj.get("type").and_then(Value::as_str) {
            Some(kind) if !kind.is_empty() => kind.to_string(),
            _ => match obj.get("protocol").and_then(Value::as_str) {
                Some(protocol) if !protocol.is_empty() => format!("xray{protocol}"),
                _ => bail!("в outbound нет ни `type`, ни `protocol`"),
            },
        };
        let name = obj
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(Self {
            kind,
            name,
            outbound,
            ..Default::default()
        })
    }

    /// Адрес сервера в виде `host:port` — для колонки в списке. У протоколов
    /// без единственной точки входа (wireguard с несколькими пирами, custom)
    /// возвращает то, что удалось найти.
    pub fn address(&self) -> String {
        if self.is_xray_native() {
            return self.xray_address();
        }
        let obj = self.outbound.as_object();
        let get = |k: &str| obj.and_then(|o| o.get(k));
        let host = get("server")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                // wireguard: сервер лежит внутри первого пира
                get("peers")
                    .and_then(Value::as_array)
                    .and_then(|p| p.first())
                    .and_then(|p| p.get("server"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let port = get("server_port")
            .and_then(Value::as_i64)
            .or_else(|| {
                get("peers")
                    .and_then(Value::as_array)
                    .and_then(|p| p.first())
                    .and_then(|p| p.get("server_port"))
                    .and_then(Value::as_i64)
            })
            .unwrap_or(0);
        match (host.is_empty(), port) {
            (true, _) => String::new(),
            (false, 0) => host,
            (false, p) if host.contains(':') => format!("[{host}]:{p}"),
            (false, p) => format!("{host}:{p}"),
        }
    }

    /// Адрес из outbound-а в формате Xray: он лежит либо в `settings`, либо в
    /// `settings.vnext[0]` — панели пишут и так, и так.
    fn xray_address(&self) -> String {
        let settings = &self.outbound["settings"];
        let node = match settings["vnext"].as_array().and_then(|v| v.first()) {
            Some(first) => first,
            None => settings,
        };
        let host = node["address"].as_str().unwrap_or_default();
        let port = node["port"].as_i64().unwrap_or(0);
        match (host.is_empty(), port) {
            (true, _) => String::new(),
            (false, 0) => host.to_string(),
            (false, p) if host.contains(':') => format!("[{host}]:{p}"),
            (false, p) => format!("{host}:{p}"),
        }
    }

    /// Копия outbound-а с проставленным тегом — под этим именем профиль
    /// попадает в конфиг и во все ответы ядра.
    pub fn tagged(&self, tag: &str) -> Value {
        let mut out = self.outbound.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("tag".into(), Value::String(tag.to_string()));
            // Профиль, разобранный из ссылки с транспортом XHTTP, хранится в
            // форме sing-box, но исполняется Xray; sing-box такого типа не
            // знает и должен видеть обычный vless.
            if self.kind == "xrayvless" && obj.contains_key("type") {
                obj.insert("type".into(), Value::String("vless".into()));
            }
        }
        out
    }

    /// Профиль исполняется Xray-ядром, а не sing-box.
    pub fn is_xray(&self) -> bool {
        self.kind.starts_with("xray")
    }

    /// Outbound уже записан в формате Xray и передаётся ему как есть.
    pub fn is_xray_native(&self) -> bool {
        self.outbound.get("protocol").is_some() && self.outbound.get("type").is_none()
    }

    pub fn latency_label(&self) -> String {
        match self.latency {
            0 => "—".into(),
            l if l < 0 => "таймаут".into(),
            l => format!("{l} мс"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    /// Пусто у ручной группы, иначе — ссылка на подписку.
    pub url: String,
    pub info: String,
    pub archive: bool,
    pub skip_auto_update: bool,
    pub sub_last_update: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Group {
    pub fn is_subscription(&self) -> bool {
        !self.url.is_empty()
    }
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            url: String::new(),
            info: String::new(),
            archive: false,
            skip_auto_update: false,
            sub_last_update: 0,
            created_at: 0,
            updated_at: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn address_from_plain_outbound() {
        let p = Profile::from_outbound(json!({
            "type": "vless", "tag": "de1", "server": "de1.example.com", "server_port": 443
        }))
        .unwrap();
        assert_eq!(p.address(), "de1.example.com:443");
        assert_eq!(p.name, "de1");
    }

    #[test]
    fn address_from_wireguard_peer() {
        let p = Profile::from_outbound(json!({
            "type": "wireguard",
            "peers": [{"server": "engage.example", "server_port": 2408}]
        }))
        .unwrap();
        assert_eq!(p.address(), "engage.example:2408");
    }

    #[test]
    fn ipv6_address_is_bracketed() {
        let p = Profile::from_outbound(json!({
            "type": "vless", "server": "2001:db8::1", "server_port": 443
        }))
        .unwrap();
        assert_eq!(p.address(), "[2001:db8::1]:443");
    }

    #[test]
    fn xray_profile_downgrades_type_for_singbox() {
        let mut p = Profile::from_outbound(json!({"type": "vless", "server": "a", "server_port": 1}))
            .unwrap();
        p.kind = "xrayvless".into();
        assert_eq!(p.tagged("proxy")["type"], "vless");
        assert_eq!(p.tagged("proxy")["tag"], "proxy");
    }

    #[test]
    fn outbound_without_type_is_rejected() {
        assert!(Profile::from_outbound(json!({"server": "a"})).is_err());
    }

    #[test]
    fn xray_native_outbound_is_recognised() {
        let p = Profile::from_outbound(json!({
            "protocol": "vless",
            "settings": {"address": "h2.example", "port": 443, "id": "u"},
            "streamSettings": {"network": "xhttp", "security": "reality"}
        }))
        .unwrap();
        assert_eq!(p.kind, "xrayvless");
        assert!(p.is_xray());
        assert!(p.is_xray_native());
        assert_eq!(p.address(), "h2.example:443");
        // Тег проставляется, но структура Xray не переписывается.
        let tagged = p.tagged("proxy");
        assert_eq!(tagged["tag"], "proxy");
        assert_eq!(tagged["protocol"], "vless");
        assert!(tagged.get("type").is_none());
    }

    #[test]
    fn xray_native_with_vnext_layout() {
        let p = Profile::from_outbound(json!({
            "protocol": "vless",
            "settings": {"vnext": [{"address": "v.example", "port": 8443, "users": [{"id": "u"}]}]},
        }))
        .unwrap();
        assert_eq!(p.address(), "v.example:8443");
    }
}
