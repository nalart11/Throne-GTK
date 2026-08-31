//! Profile model. `outbound` is stored exactly in the form expected by
//! sing-box; this is both the storage format and what is sent to the core. No
//! intermediate protocol representation is used: any new sing-box protocol
//! starts working without changes to this crate.

use anyhow::{bail, Result};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: i64,
    /// sing-box outbound type: vless, vmess, shadowsocks, trojan, hysteria2, …
    /// Plus our own types: `xrayvless` and `custom`.
    pub kind: String,
    pub name: String,
    pub gid: i64,
    /// Last latency in ms. 0 means untested, <0 means the test failed.
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
        // Two native formats: sing-box (`type`) and Xray (`protocol` +
        // `streamSettings`). The Xray form comes from Throne databases and panels
        // distributing ready-made configurations; converting it to sing-box loses
        // xmux, downloadSettings, and other extensions that have no sing-box equivalent.
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

    /// Server address in `host:port` form for the list column. For protocols
    /// without a single endpoint (wireguard with multiple peers, custom),
    /// returns whatever could be found.
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
                // wireguard: the server is inside the first peer
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

    /// Address from an Xray-format outbound: it is either in `settings` or in
    /// `settings.vnext[0]`; panels use both layouts.
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

    /// A copy of the outbound with its tag set; under this name the profile
    /// appears in the configuration and all core responses.
    pub fn tagged(&self, tag: &str) -> Value {
        let mut out = self.outbound.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("tag".into(), Value::String(tag.to_string()));
            // A profile parsed from a link with XHTTP transport is stored in
            // sing-box form but runs in Xray; sing-box does not know this type
            // and must see ordinary vless.
            if self.kind == "xrayvless" && obj.contains_key("type") {
                obj.insert("type".into(), Value::String("vless".into()));
            }
        }
        out
    }

    /// The profile is executed by the Xray core, not sing-box.
    pub fn is_xray(&self) -> bool {
        self.kind.starts_with("xray")
    }

    /// The outbound is already in Xray format and is passed through unchanged.
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    pub id: i64,
    pub name: String,
    /// Empty for a manual group; otherwise, the subscription link.
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
        let mut p =
            Profile::from_outbound(json!({"type": "vless", "server": "a", "server_port": 1}))
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
        // The tag is set, but the Xray structure is not rewritten.
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
