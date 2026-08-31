//! Storage: groups, profiles, and settings in SQLite.
//!
//! A separate database (`~/.config/throne-gtk/`), rather than one shared with the
//! original Throne: two programs writing to one file will eventually overwrite
//! each other's changes. Data from Throne is moved by a one-time import — see [`Store::import_throne`].

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use throne_config::profile::{Group, Profile};
use throne_config::route::{Condition, MatchKind, RouteAction, RouteRule};
use throne_config::Settings;

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS groups (
    id                INTEGER PRIMARY KEY,
    name              TEXT    NOT NULL DEFAULT '',
    url               TEXT    NOT NULL DEFAULT '',
    info              TEXT    NOT NULL DEFAULT '',
    archive           INTEGER NOT NULL DEFAULT 0,
    skip_auto_update  INTEGER NOT NULL DEFAULT 0,
    sub_last_update   INTEGER NOT NULL DEFAULT 0,
    display_order     INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at        INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);

CREATE TABLE IF NOT EXISTS profiles (
    id            INTEGER PRIMARY KEY,
    type          TEXT    NOT NULL,
    name          TEXT    NOT NULL DEFAULT '',
    gid           INTEGER NOT NULL DEFAULT 0,
    latency       INTEGER NOT NULL DEFAULT 0,
    dl_speed      TEXT    NOT NULL DEFAULT '',
    ul_speed      TEXT    NOT NULL DEFAULT '',
    test_country  TEXT    NOT NULL DEFAULT '',
    ip_out        TEXT    NOT NULL DEFAULT '',
    outbound_json TEXT    NOT NULL,
    traffic_dl    INTEGER NOT NULL DEFAULT 0,
    traffic_up    INTEGER NOT NULL DEFAULT 0,
    latency_at    INTEGER NOT NULL DEFAULT 0,
    display_order INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at    INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    FOREIGN KEY (gid) REFERENCES groups(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_profiles_gid ON profiles(gid);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("создание каталога {}", dir.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("открытие базы {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // WAL survives a process crash without losing the last transaction;
        // foreign_keys is needed so deleting a group also deletes its profiles.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let store = Self { conn };
        store.ensure_default_group()?;
        Ok(store)
    }

    /// The group with ID 1 always exists: manually added profiles go into it,
    /// and the import uses it when the source data has no group.
    fn ensure_default_group(&self) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO groups (id, name) VALUES (1, 'Мои серверы')",
            [],
        )?;
        Ok(())
    }

    // ── groups ──────────────────────────────────────────────────────────

    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, url, info, archive, skip_auto_update, sub_last_update,
                    created_at, updated_at
             FROM groups ORDER BY display_order, id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok(row_to_group(r)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn group(&self, id: i64) -> Result<Option<Group>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, url, info, archive, skip_auto_update, sub_last_update,
                    created_at, updated_at
             FROM groups WHERE id = ?1",
        )?;
        Ok(stmt.query_row([id], |r| Ok(row_to_group(r))).optional()?)
    }

    pub fn insert_group(&self, group: &Group) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO groups (name, url, info, archive, skip_auto_update, sub_last_update,
                                 display_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                     (SELECT COALESCE(MAX(display_order), 0) + 1 FROM groups))",
            params![
                group.name,
                group.url,
                group.info,
                group.archive as i64,
                group.skip_auto_update as i64,
                group.sub_last_update,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_group(&self, group: &Group) -> Result<()> {
        self.conn.execute(
            "UPDATE groups SET name = ?2, url = ?3, info = ?4, archive = ?5,
                    skip_auto_update = ?6, sub_last_update = ?7,
                    updated_at = strftime('%s','now')
             WHERE id = ?1",
            params![
                group.id,
                group.name,
                group.url,
                group.info,
                group.archive as i64,
                group.skip_auto_update as i64,
                group.sub_last_update,
            ],
        )?;
        Ok(())
    }

    /// Deletes a group together with its profiles. Leaves the default group alone;
    /// otherwise new profiles would have nowhere to go.
    pub fn delete_group(&self, id: i64) -> Result<()> {
        if id == 1 {
            anyhow::bail!("группу по умолчанию удалить нельзя");
        }
        self.conn
            .execute("DELETE FROM groups WHERE id = ?1", [id])?;
        Ok(())
    }

    // ── profiles ─────────────────────────────────────────────────────────

    pub fn profiles(&self, gid: Option<i64>) -> Result<Vec<Profile>> {
        let (sql, params) = match gid {
            Some(gid) => (
                "SELECT id, type, name, gid, latency, dl_speed, ul_speed, test_country, ip_out,
                        outbound_json, traffic_dl, traffic_up, created_at, updated_at, latency_at
                 FROM profiles WHERE gid = ?1 ORDER BY display_order, id",
                vec![gid],
            ),
            None => (
                "SELECT id, type, name, gid, latency, dl_speed, ul_speed, test_country, ip_out,
                        outbound_json, traffic_dl, traffic_up, created_at, updated_at, latency_at
                 FROM profiles ORDER BY gid, display_order, id",
                vec![],
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok(row_to_profile(r))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn profile(&self, id: i64) -> Result<Option<Profile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, type, name, gid, latency, dl_speed, ul_speed, test_country, ip_out,
                    outbound_json, traffic_dl, traffic_up, created_at, updated_at, latency_at
             FROM profiles WHERE id = ?1",
        )?;
        Ok(stmt.query_row([id], |r| Ok(row_to_profile(r))).optional()?)
    }

    pub fn insert_profile(&self, profile: &Profile) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO profiles (type, name, gid, outbound_json, latency, dl_speed, ul_speed,
                                   test_country, ip_out, traffic_dl, traffic_up, latency_at,
                                   display_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     (SELECT COALESCE(MAX(display_order), 0) + 1 FROM profiles WHERE gid = ?3))",
            params![
                profile.kind,
                profile.name,
                profile.gid,
                profile.outbound.to_string(),
                profile.latency,
                profile.dl_speed,
                profile.ul_speed,
                profile.test_country,
                profile.ip_out,
                profile.traffic_dl,
                profile.traffic_up,
                profile.latency_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_profile(&self, profile: &Profile) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET type = ?2, name = ?3, gid = ?4, outbound_json = ?5,
                    latency = ?6, dl_speed = ?7, ul_speed = ?8, test_country = ?9, ip_out = ?10,
                    traffic_dl = ?11, traffic_up = ?12, latency_at = ?13,
                    updated_at = strftime('%s','now')
             WHERE id = ?1",
            params![
                profile.id,
                profile.kind,
                profile.name,
                profile.gid,
                profile.outbound.to_string(),
                profile.latency,
                profile.dl_speed,
                profile.ul_speed,
                profile.test_country,
                profile.ip_out,
                profile.traffic_dl,
                profile.traffic_up,
                profile.latency_at,
            ],
        )?;
        Ok(())
    }

    /// Stores only the test result — called in batches after a run and does not
    /// touch fields that the user may have edited concurrently.
    pub fn set_latency(&self, id: i64, latency: i32, at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET latency = ?2, latency_at = ?3 WHERE id = ?1",
            params![id, latency, at],
        )?;
        Ok(())
    }

    /// Speed measurement result. An empty string means “not measured”; preserve it
    /// as is so it is not confused with zero speed.
    pub fn set_speed(&self, id: i64, dl: &str, ul: &str, country: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET dl_speed = ?2, ul_speed = ?3, test_country = ?4 WHERE id = ?1",
            params![id, dl, ul, country],
        )?;
        Ok(())
    }

    pub fn add_traffic(&self, id: i64, dl: i64, up: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET traffic_dl = traffic_dl + ?2, traffic_up = traffic_up + ?3
             WHERE id = ?1",
            params![id, dl, up],
        )?;
        Ok(())
    }

    pub fn delete_profile(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_profiles(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Replaces the contents of a group with the result of a subscription update.
    ///
    /// Servers matching by “fingerprint” retain their accumulated latency and
    /// traffic: subscriptions resend the same list every few hours, and without
    /// this, test history would be reset on every update.
    /// Returns (added, kept, removed).
    pub fn replace_group_profiles(
        &mut self,
        gid: i64,
        incoming: &[Profile],
    ) -> Result<(usize, usize, usize)> {
        let existing = self.profiles(Some(gid))?;
        let mut kept = 0usize;

        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM profiles WHERE gid = ?1", [gid])?;

        for (order, profile) in incoming.iter().enumerate() {
            let previous = existing
                .iter()
                .find(|old| fingerprint(old) == fingerprint(profile));
            if previous.is_some() {
                kept += 1;
            }
            tx.execute(
                "INSERT INTO profiles (type, name, gid, outbound_json, latency, dl_speed, ul_speed,
                                       test_country, ip_out, traffic_dl, traffic_up, latency_at,
                                       display_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    profile.kind,
                    profile.name,
                    gid,
                    profile.outbound.to_string(),
                    previous.map_or(0, |p| p.latency),
                    previous.map_or(String::new(), |p| p.dl_speed.clone()),
                    previous.map_or(String::new(), |p| p.ul_speed.clone()),
                    previous.map_or(String::new(), |p| p.test_country.clone()),
                    previous.map_or(String::new(), |p| p.ip_out.clone()),
                    previous.map_or(0, |p| p.traffic_dl),
                    previous.map_or(0, |p| p.traffic_up),
                    previous.map_or(0, |p| p.latency_at),
                    order as i64,
                ],
            )?;
        }
        tx.commit()?;

        Ok((
            incoming.len().saturating_sub(kept),
            kept,
            existing.len().saturating_sub(kept),
        ))
    }

    // ── settings ───────────────────────────────────────────────────────

    /// Settings are stored under one key per field: adding a new field therefore
    /// requires no migration, and unknown keys from future versions do not bother old ones.
    pub fn settings(&self) -> Result<Settings> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings")?;
        let mut map = serde_json::Map::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (key, raw) = row?;
            let value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
            map.insert(key, value);
        }
        Ok(serde_json::from_value(Value::Object(map)).unwrap_or_default())
    }

    pub fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        let Value::Object(map) = serde_json::to_value(settings)? else {
            anyhow::bail!("настройки должны сериализоваться в объект");
        };
        let tx = self.conn.transaction()?;
        for (key, value) in map {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── import from the original Throne ──────────────────────────────────

    /// Imports groups, profiles, and routing rules from the Throne database.
    ///
    /// Returns counts of imported items — see [`ImportOutcome`].
    pub fn import_throne(&mut self, throne_db: &Path) -> Result<ImportOutcome> {
        let src = open_throne(throne_db)?;

        let mut groups = src.prepare(
            "SELECT id, name, COALESCE(url,''), COALESCE(info,''), archive, skip_auto_update,
                    sub_last_update
             FROM groups ORDER BY id",
        )?;
        let incoming_groups: Vec<(i64, String, String, String, i64, i64, i64)> = groups
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut group_count = 0;
        let mut profile_count = 0;

        for (old_id, name, url, info, archive, skip, last_update) in incoming_groups {
            // Throne stores the subscription header as is; there is no reason to
            // show `upload=0; download=0; expire=...` to the user.
            let info = match info.contains('=') {
                true => throne_config::subscription::format_userinfo(&info),
                false => info,
            };
            let new_gid = if old_id == 1 {
                // Throne's first group is the same default group; no separate copy is needed.
                1
            } else {
                self.insert_group(&Group {
                    name,
                    url,
                    info,
                    archive: archive != 0,
                    skip_auto_update: skip != 0,
                    sub_last_update: last_update,
                    ..Default::default()
                })?
            };
            group_count += 1;

            let mut profiles = src.prepare(
                "SELECT type, COALESCE(name,''), latency, COALESCE(dl_speed,''),
                        COALESCE(ul_speed,''), COALESCE(test_country,''), COALESCE(ip_out,''),
                        outbound_json, traffic_dl, traffic_up, latency_at
                 FROM profiles WHERE gid = ?1 ORDER BY id",
            )?;
            let rows = profiles.query_map([old_id], |r| {
                Ok(Profile {
                    kind: r.get(0)?,
                    name: r.get(1)?,
                    latency: r.get(2)?,
                    dl_speed: r.get(3)?,
                    ul_speed: r.get(4)?,
                    test_country: r.get(5)?,
                    ip_out: r.get(6)?,
                    outbound: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(Value::Null),
                    traffic_dl: r.get(8)?,
                    traffic_up: r.get(9)?,
                    latency_at: r.get(10)?,
                    gid: new_gid,
                    ..Default::default()
                })
            })?;

            for profile in rows {
                let profile = profile?;
                // A profile without a parseable outbound is useless: it cannot be
                // assembled into a configuration and will fail silently on connection.
                if !profile.outbound.is_object() {
                    tracing::warn!("пропускаю профиль `{}`: битый outbound", profile.name);
                    continue;
                }
                self.insert_profile(&profile)?;
                profile_count += 1;
            }
        }

        let (rules, skipped_rules) = self.import_route_rules(&src)?;

        Ok(ImportOutcome {
            groups: group_count,
            profiles: profile_count,
            rules,
            skipped_rules,
        })
    }

    /// Imports only the routing rules. The servers are usually already in place by
    /// this point, and a full import would add duplicate copies of their groups.
    ///
    /// Returns (imported rules, skipped Throne rules).
    pub fn import_throne_routes(&mut self, throne_db: &Path) -> Result<(usize, usize)> {
        let src = open_throne(throne_db)?;
        self.import_route_rules(&src)
    }

    /// Routing rules of the active Throne profile.
    ///
    /// The rule is imported in full, with all conditions, and receives the
    /// “any match” mode. In Throne, such a rule would mean “domain or subnet,
    /// and also a ready-made list” — the “and” silently cancels part of the list,
    /// while mixed rules are written as a list of exceptions.
    ///
    /// Returns (imported rules, skipped Throne rules).
    fn import_route_rules(&mut self, src: &Connection) -> Result<(usize, usize)> {
        // Rules were not present in the first version of Throne: a database created
        // before them is imported as before — by groups and profiles.
        if !table_exists(src, "route_rules")? {
            return Ok((0, 0));
        }

        // Throne may have several routing profiles; the one selected in the
        // interface is used.
        let active: i64 = match table_exists(src, "settings")? {
            true => src
                .query_row(
                    "SELECT value FROM settings WHERE key = 'current_route_id'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?,
            false => None,
        }
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1);

        let mut stmt = src.prepare(
            "SELECT COALESCE(name,''), domain_json, domain_suffix_json, domain_keyword_json,
                    domain_regex_json, ip_cidr_json, port_json, port_range_json,
                    process_name_json, rule_set_json,
                    ip_version, network, protocol, inbound_json, source_ip_cidr_json,
                    source_port_json, source_port_range_json, process_path_json,
                    process_path_regex_json, wifi_ssid_json, wifi_bssid_json,
                    source_ip_is_private, ip_is_private, invert, outbound_id, action
             FROM route_rules WHERE route_profile_id = ?1 ORDER BY rule_order",
        )?;
        let rows = stmt
            .query_map([active], |r| {
                Ok(ThroneRule {
                    name: r.get(0)?,
                    conditions: [
                        (MatchKind::Domain, json_values(r.get(1)?)),
                        (MatchKind::DomainSuffix, json_values(r.get(2)?)),
                        (MatchKind::DomainKeyword, json_values(r.get(3)?)),
                        (MatchKind::DomainRegex, json_values(r.get(4)?)),
                        (MatchKind::IpCidr, json_values(r.get(5)?)),
                        (MatchKind::Port, {
                            let mut ports = json_values(r.get(6)?);
                            ports.extend(json_values(r.get(7)?));
                            ports
                        }),
                        (MatchKind::Process, json_values(r.get(8)?)),
                        (MatchKind::RuleSet, json_values(r.get(9)?)),
                    ],
                    // Conditions that we do not support. A rule with such a
                    // condition cannot be imported: without it, it would catch too much.
                    unsupported: [
                        r.get::<_, Option<String>>(10)?
                            .is_some_and(|v| !v.trim().is_empty()),
                        r.get::<_, Option<String>>(11)?
                            .is_some_and(|v| !v.trim().is_empty()),
                        r.get::<_, Option<String>>(12)?
                            .is_some_and(|v| !v.trim().is_empty()),
                        !json_values(r.get(13)?).is_empty(),
                        !json_values(r.get(14)?).is_empty(),
                        !json_values(r.get(15)?).is_empty(),
                        !json_values(r.get(16)?).is_empty(),
                        !json_values(r.get(17)?).is_empty(),
                        !json_values(r.get(18)?).is_empty(),
                        !json_values(r.get(19)?).is_empty(),
                        !json_values(r.get(20)?).is_empty(),
                        r.get::<_, i64>(21)? != 0,
                        r.get::<_, i64>(22)? != 0,
                        r.get::<_, i64>(23)? != 0,
                    ]
                    .iter()
                    .any(|used| *used),
                    outbound_id: r.get(24)?,
                    action: r.get(25)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut imported = Vec::new();
        let mut skipped = 0;
        for rule in rows {
            match rule.to_route_rule() {
                Some(rule) => imported.push(rule),
                None => skipped += 1,
            }
        }

        if imported.is_empty() {
            return Ok((0, skipped));
        }
        // Import adds rather than replaces: the user may already have created
        // their own rules, and those come first.
        let count = imported.len();
        let mut settings = self.settings()?;
        settings.route_rules.extend(imported);
        self.save_settings(&settings)?;
        Ok((count, skipped))
    }
}

/// Counts of items imported from Throne.
pub struct ImportOutcome {
    pub groups: usize,
    pub profiles: usize,
    pub rules: usize,
    /// Throne rules that could not be imported: special rules (its core sets up
    /// DNS interception itself), chains to a specific server, and conditions not
    /// supported by our model — by connection source, by network, or inverted.
    pub skipped_rules: usize,
}

/// A routing rule in the form in which Throne stores it.
struct ThroneRule {
    name: String,
    conditions: [(MatchKind, Vec<String>); 8],
    unsupported: bool,
    outbound_id: i64,
    action: String,
}

impl ThroneRule {
    /// Converts a Throne rule to our model. `None` means the rule is not imported.
    fn to_route_rule(&self) -> Option<RouteRule> {
        if self.unsupported {
            return None;
        }
        let action = self.route_action()?;
        let conditions: Vec<Condition> = self
            .conditions
            .iter()
            .filter(|(_, values)| !values.is_empty())
            .map(|(kind, values)| Condition::new(*kind, values.clone()))
            .collect();
        if conditions.is_empty() {
            return None;
        }

        Some(RouteRule {
            name: self.name.trim().to_string(),
            conditions,
            action,
            ..Default::default()
        })
    }

    /// Where the rule sends traffic. `None` means a destination we do not support:
    /// DNS interception, a chain through a specific server, or sniff sorting.
    fn route_action(&self) -> Option<RouteAction> {
        match self.action.as_str() {
            "reject" => Some(RouteAction::Block),
            // Throne keeps the destination in a separate field, while its `action`
            // remains "route": -1 proxy, -2 direct, -3 block, -4 DNS interception.
            // A value of zero or higher is a chain to a specific profile.
            "route" => match self.outbound_id {
                -1 => Some(RouteAction::Proxy),
                -2 => Some(RouteAction::Direct),
                -3 => Some(RouteAction::Block),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The Throne database is opened read-only: the original must remain untouched
/// by the import, even if it is currently running.
fn open_throne(path: &Path) -> Result<Connection> {
    if !path.exists() {
        anyhow::bail!("файл {} не найден", path.display());
    }
    Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("открытие {}", path.display()))
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let found: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// Condition values: Throne stores them as a JSON array of strings or numbers.
fn json_values(raw: Option<String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .filter(|v| !v.is_empty())
        .collect()
}

/// Server fingerprint without the name and tag: subscriptions rename servers
/// (traffic counters and dates in the name), but the address and keys remain the same.
fn fingerprint(profile: &Profile) -> String {
    let mut copy = profile.outbound.clone();
    if let Some(obj) = copy.as_object_mut() {
        obj.remove("tag");
    }
    copy.to_string()
}

fn row_to_group(r: &Row<'_>) -> Group {
    Group {
        id: r.get_unwrap(0),
        name: r.get_unwrap(1),
        url: r.get_unwrap(2),
        info: r.get_unwrap(3),
        archive: r.get_unwrap::<_, i64>(4) != 0,
        skip_auto_update: r.get_unwrap::<_, i64>(5) != 0,
        sub_last_update: r.get_unwrap(6),
        created_at: r.get_unwrap(7),
        updated_at: r.get_unwrap(8),
    }
}

fn row_to_profile(r: &Row<'_>) -> Profile {
    Profile {
        id: r.get_unwrap(0),
        kind: r.get_unwrap(1),
        name: r.get_unwrap(2),
        gid: r.get_unwrap(3),
        latency: r.get_unwrap(4),
        dl_speed: r.get_unwrap(5),
        ul_speed: r.get_unwrap(6),
        test_country: r.get_unwrap(7),
        ip_out: r.get_unwrap(8),
        outbound: serde_json::from_str(&r.get_unwrap::<_, String>(9)).unwrap_or(Value::Null),
        traffic_dl: r.get_unwrap(10),
        traffic_up: r.get_unwrap(11),
        created_at: r.get_unwrap(12),
        updated_at: r.get_unwrap(13),
        latency_at: r.get_unwrap(14),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use throne_config::link;
    use throne_config::route::MatchMode;

    fn profile(link_str: &str) -> Profile {
        link::parse(link_str).unwrap()
    }

    #[test]
    fn profiles_round_trip() {
        let store = Store::open_memory().unwrap();
        let mut p = profile("vless://uuid@a.example:443?security=tls&sni=a.example#a");
        p.gid = 1;
        let id = store.insert_profile(&p).unwrap();

        let loaded = store.profile(id).unwrap().unwrap();
        assert_eq!(loaded.name, "a");
        assert_eq!(loaded.outbound, p.outbound);
        assert_eq!(loaded.gid, 1);
    }

    #[test]
    fn speed_result_is_stored() {
        let store = Store::open_memory().unwrap();
        let mut p = profile("trojan://pw@a.example:443#a");
        p.gid = 1;
        let id = store.insert_profile(&p).unwrap();

        store.set_speed(id, "25.4 МБ/с", "", "DE").unwrap();
        let loaded = store.profile(id).unwrap().unwrap();
        assert_eq!(loaded.dl_speed, "25.4 МБ/с");
        assert_eq!(loaded.ul_speed, "");
        assert_eq!(loaded.test_country, "DE");
    }

    #[test]
    fn deleting_a_group_deletes_its_profiles() {
        let store = Store::open_memory().unwrap();
        let gid = store
            .insert_group(&Group {
                name: "sub".into(),
                url: "https://example/sub".into(),
                ..Default::default()
            })
            .unwrap();
        let mut p = profile("trojan://pw@t.example:443#t");
        p.gid = gid;
        store.insert_profile(&p).unwrap();
        assert_eq!(store.profiles(Some(gid)).unwrap().len(), 1);

        store.delete_group(gid).unwrap();
        assert_eq!(store.profiles(Some(gid)).unwrap().len(), 0);
    }

    #[test]
    fn default_group_cannot_be_deleted() {
        let store = Store::open_memory().unwrap();
        assert!(store.delete_group(1).is_err());
    }

    #[test]
    fn subscription_update_keeps_latency_of_unchanged_servers() {
        let mut store = Store::open_memory().unwrap();
        let gid = store
            .insert_group(&Group {
                name: "sub".into(),
                url: "u".into(),
                ..Default::default()
            })
            .unwrap();

        let mut old = profile("trojan://pw@a.example:443#старое имя");
        old.gid = gid;
        let id = store.insert_profile(&old).unwrap();
        store.set_latency(id, 42, 1700).unwrap();
        store.add_traffic(id, 1000, 500).unwrap();

        // The subscription sent the same server with a new name and one new server.
        let incoming = vec![
            profile("trojan://pw@a.example:443#новое имя"),
            profile("trojan://pw@b.example:443#второй"),
        ];
        let (added, kept, removed) = store.replace_group_profiles(gid, &incoming).unwrap();
        assert_eq!((added, kept, removed), (1, 1, 0));

        let profiles = store.profiles(Some(gid)).unwrap();
        assert_eq!(profiles.len(), 2);
        let same = profiles.iter().find(|p| p.name == "новое имя").unwrap();
        assert_eq!(same.latency, 42, "задержка должна пережить обновление");
        assert_eq!(same.traffic_dl, 1000);
        let fresh = profiles.iter().find(|p| p.name == "второй").unwrap();
        assert_eq!(fresh.latency, 0);
    }

    #[test]
    fn subscription_update_reports_removals() {
        let mut store = Store::open_memory().unwrap();
        let mut a = profile("trojan://pw@a.example:443#a");
        a.gid = 1;
        store.insert_profile(&a).unwrap();
        let mut b = profile("trojan://pw@b.example:443#b");
        b.gid = 1;
        store.insert_profile(&b).unwrap();

        let incoming = vec![profile("trojan://pw@a.example:443#a")];
        let (added, kept, removed) = store.replace_group_profiles(1, &incoming).unwrap();
        assert_eq!((added, kept, removed), (0, 1, 1));
        assert_eq!(store.profiles(Some(1)).unwrap().len(), 1);
    }

    #[test]
    fn settings_round_trip_and_defaults() {
        let mut store = Store::open_memory().unwrap();
        assert_eq!(store.settings().unwrap(), Settings::default());

        let mut s = Settings::default();
        s.mixed_port = 1080;
        s.mode = throne_config::settings::ProxyMode::Vpn;
        s.dns_remote = "https://dns.google/dns-query".into();
        store.save_settings(&s).unwrap();

        let loaded = store.settings().unwrap();
        assert_eq!(loaded.mixed_port, 1080);
        assert!(loaded.mode.is_vpn());
        assert_eq!(loaded.dns_remote, "https://dns.google/dns-query");
    }

    #[test]
    fn unknown_settings_keys_are_ignored() {
        let store = Store::open_memory().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('из_будущей_версии', '\"x\"')",
                [],
            )
            .unwrap();
        assert_eq!(store.settings().unwrap().mixed_port, 2080);
    }

    /// The import reads the Throne database as is — including records that our code
    /// would not create itself.
    #[test]
    fn import_from_throne_layout() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("throne.db");
        {
            let src = Connection::open(&src_path).unwrap();
            src.execute_batch(
                r#"
                CREATE TABLE groups (id INTEGER PRIMARY KEY, archive INTEGER NOT NULL DEFAULT 0,
                    skip_auto_update INTEGER NOT NULL DEFAULT 0, name TEXT NOT NULL DEFAULT '',
                    url TEXT, info TEXT, sub_last_update INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE profiles (id INTEGER PRIMARY KEY, type TEXT NOT NULL, name TEXT,
                    gid INTEGER NOT NULL DEFAULT 0, latency INTEGER NOT NULL DEFAULT 0,
                    dl_speed TEXT, ul_speed TEXT, test_country TEXT, ip_out TEXT,
                    outbound_json TEXT NOT NULL, traffic_dl INTEGER NOT NULL DEFAULT 0,
                    traffic_up INTEGER NOT NULL DEFAULT 0, latency_at INTEGER NOT NULL DEFAULT 0);
                INSERT INTO groups (id, name, url) VALUES (1, 'Default', '');
                INSERT INTO groups (id, name, url) VALUES (3, 'xfizz', 'https://x/sub');
                INSERT INTO profiles (type, name, gid, latency, outbound_json)
                    VALUES ('vless', 'DE #1', 3, 55,
                            '{"type":"vless","server":"de.example","server_port":443,"uuid":"u"}');
                INSERT INTO profiles (type, name, gid, outbound_json)
                    VALUES ('vless', 'битый', 3, 'не json');
                "#,
            )
            .unwrap();
        }

        let mut store = Store::open_memory().unwrap();
        let done = store.import_throne(&src_path).unwrap();
        assert_eq!(done.groups, 2);
        assert_eq!(done.profiles, 1, "профиль с битым outbound не переносится");
        assert_eq!(
            done.rules, 0,
            "в базе без таблиц маршрутизации нечего переносить"
        );

        let imported = store.groups().unwrap();
        assert!(imported
            .iter()
            .any(|g| g.name == "xfizz" && g.is_subscription()));
        let all = store.profiles(None).unwrap();
        assert_eq!(all[0].name, "DE #1");
        assert_eq!(all[0].latency, 55);
    }

    /// A Throne rule is imported together with all of its conditions.
    #[test]
    fn import_carries_throne_route_rules() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("throne.db");
        {
            let src = Connection::open(&src_path).unwrap();
            src.execute_batch(
                r#"
                CREATE TABLE groups (id INTEGER PRIMARY KEY, archive INTEGER NOT NULL DEFAULT 0,
                    skip_auto_update INTEGER NOT NULL DEFAULT 0, name TEXT NOT NULL DEFAULT '',
                    url TEXT, info TEXT, sub_last_update INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE profiles (id INTEGER PRIMARY KEY, type TEXT NOT NULL, name TEXT,
                    gid INTEGER NOT NULL DEFAULT 0, latency INTEGER NOT NULL DEFAULT 0,
                    dl_speed TEXT, ul_speed TEXT, test_country TEXT, ip_out TEXT,
                    outbound_json TEXT NOT NULL, traffic_dl INTEGER NOT NULL DEFAULT 0,
                    traffic_up INTEGER NOT NULL DEFAULT 0, latency_at INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE route_rules (route_profile_id INTEGER NOT NULL,
                    rule_order INTEGER NOT NULL, name TEXT NOT NULL DEFAULT '',
                    ip_version TEXT, network TEXT, protocol TEXT, inbound_json TEXT,
                    domain_json TEXT, domain_suffix_json TEXT, domain_keyword_json TEXT,
                    domain_regex_json TEXT, source_ip_cidr_json TEXT,
                    source_ip_is_private INTEGER NOT NULL DEFAULT 0, ip_cidr_json TEXT,
                    ip_is_private INTEGER NOT NULL DEFAULT 0, source_port_json TEXT,
                    source_port_range_json TEXT, port_json TEXT, port_range_json TEXT,
                    process_name_json TEXT, process_path_json TEXT,
                    process_path_regex_json TEXT, rule_set_json TEXT,
                    invert INTEGER NOT NULL DEFAULT 0, outbound_id INTEGER NOT NULL DEFAULT -2,
                    action TEXT NOT NULL DEFAULT 'route', wifi_ssid_json TEXT,
                    wifi_bssid_json TEXT);
                INSERT INTO groups (id, name, url) VALUES (1, 'Default', '');
                INSERT INTO settings (key, value) VALUES ('current_route_id', '2');
                -- Перехват DNS ставит наше ядро: правило Throne не нужно.
                INSERT INTO route_rules (route_profile_id, rule_order, name, protocol,
                        outbound_id, action)
                    VALUES (2, 0, 'Route DNS', 'dns', -2, 'hijack-dns');
                -- Домены, подсети и готовые списки в одном правиле.
                INSERT INTO route_rules (route_profile_id, rule_order, name, domain_json,
                        ip_cidr_json, rule_set_json, outbound_id, action)
                    VALUES (2, 1, 'Bypass', '["mos.ru"]', '["10.0.0.0/8"]',
                            '["geoip-ru"]', -2, 'route');
                -- Условия по источнику наша модель не выражает.
                INSERT INTO route_rules (route_profile_id, rule_order, name,
                        source_ip_cidr_json, outbound_id, action)
                    VALUES (2, 2, 'Из локалки', '["192.168.0.0/16"]', -1, 'route');
                INSERT INTO route_rules (route_profile_id, rule_order, name, domain_json,
                        outbound_id, action)
                    VALUES (2, 3, 'Реклама', '["ads.example"]', -3, 'route');
                -- Правила чужого профиля не трогаем.
                INSERT INTO route_rules (route_profile_id, rule_order, name, domain_json,
                        outbound_id, action)
                    VALUES (7, 0, 'Другой профиль', '["other.example"]', -1, 'route');
                "#,
            )
            .unwrap();
        }

        let mut store = Store::open_memory().unwrap();
        let done = store.import_throne(&src_path).unwrap();
        assert_eq!(done.rules, 2, "Bypass и правило-блокировка");
        assert_eq!(done.skipped_rules, 2, "перехват DNS и правило по источнику");

        let rules = store.settings().unwrap().route_rules;
        assert_eq!(rules[0].name, "Bypass");
        let kinds: Vec<MatchKind> = rules[0].conditions.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![MatchKind::Domain, MatchKind::IpCidr, MatchKind::RuleSet]
        );
        assert_eq!(rules[0].conditions[0].values, vec!["mos.ru"]);
        assert_eq!(rules[0].match_mode, MatchMode::Any);
        assert_eq!(rules[0].action, RouteAction::Direct);
        assert_eq!(rules[1].name, "Реклама");
        assert_eq!(rules[1].action, RouteAction::Block);
    }

    #[test]
    fn import_of_missing_file_is_an_error() {
        let mut store = Store::open_memory().unwrap();
        assert!(store.import_throne(Path::new("/nope/throne.db")).is_err());
    }
}
