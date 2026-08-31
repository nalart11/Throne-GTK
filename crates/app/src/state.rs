//! Application state shared by all window pages.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use anyhow::Result;
use throne_config::{Profile, Settings};
use throne_store::Store;

use crate::engine::{Command, Engine, Status};

/// Number of log lines kept in memory. At debug level the core writes often,
/// and without a limit the window would consume hundreds of megabytes per hour.
const LOG_LIMIT: usize = 2000;

pub struct State {
    pub store: RefCell<Store>,
    pub settings: RefCell<Settings>,
    pub engine: Engine,
    pub status: RefCell<Status>,
    /// Group currently open in the server list.
    pub current_gid: Cell<i64>,
    /// Profile selected in the list (not necessarily connected).
    pub selected_id: Cell<i64>,
    pub log: RefCell<VecDeque<String>>,
}

impl State {
    pub fn new(store: Store, engine: Engine, cache_file: String) -> Result<Rc<Self>> {
        let mut settings = store.settings()?;
        settings.cache_file = cache_file;
        let selected = settings.last_profile_id;
        let gid = starting_group(&store, selected);
        Ok(Rc::new(Self {
            store: RefCell::new(store),
            settings: RefCell::new(settings),
            engine,
            status: RefCell::new(Status::Disconnected),
            current_gid: Cell::new(gid),
            selected_id: Cell::new(selected),
            log: RefCell::new(VecDeque::new()),
        }))
    }

    pub fn is_connected(&self) -> bool {
        matches!(*self.status.borrow(), Status::Connected { .. })
    }

    pub fn connected_id(&self) -> i64 {
        match *self.status.borrow() {
            Status::Connected { profile_id, .. } => profile_id,
            _ => 0,
        }
    }

    /// Whether auto-selection is selected. A negative identifier means “the
    /// entire group”: real profiles never have such IDs.
    pub fn auto_selected(&self) -> Option<i64> {
        match self.selected_id.get() {
            id if id < 0 => Some(-id),
            _ => None,
        }
    }

    pub fn selected_profile(&self) -> Option<Profile> {
        let id = self.selected_id.get();
        if id == 0 {
            return None;
        }
        self.store.borrow().profile(id).ok().flatten()
    }

    /// Saves settings and remembers the selected profile so the next launch
    /// opens where the previous one closed.
    /// The cache path is not stored in the database — it is marked `serde(skip)`.
    /// Reloads settings from the database. Needed after writing to the database
    /// outside the window — for example, when importing rules: otherwise the
    /// window keeps showing stale data, and the first save overwrites the other
    /// record with its own copy.
    pub fn reload_settings(&self) -> Result<()> {
        // cache_file is set by startup paths and is not meaningful in the database:
        // reloading would erase it.
        let cache_file = self.settings.borrow().cache_file.clone();
        let mut settings = self.store.borrow().settings()?;
        settings.cache_file = cache_file;
        *self.settings.borrow_mut() = settings;
        Ok(())
    }

    pub fn save_settings(&self) -> Result<()> {
        let mut settings = self.settings.borrow().clone();
        settings.last_profile_id = self.selected_id.get();
        self.store.borrow_mut().save_settings(&settings)?;
        // Reloading from the database would erase cache_file, so keep a copy
        // unchanged.
        *self.settings.borrow_mut() = settings;
        Ok(())
    }

    pub fn push_log(&self, line: String) {
        let mut log = self.log.borrow_mut();
        if log.len() >= LOG_LIMIT {
            log.pop_front();
        }
        log.push_back(line);
    }

    pub fn connect_selected(&self) {
        if let Some(gid) = self.auto_selected() {
            self.connect_auto(gid);
            return;
        }
        let Some(profile) = self.selected_profile() else {
            return;
        };
        self.engine.send(Command::Connect {
            profile: Box::new(profile),
            settings: Box::new(self.settings.borrow().clone()),
        });
    }

    /// Connect with group auto-selection. Servers are returned sorted by latency:
    /// the core treats this order as the initial ranking, making the first choice
    /// meaningful even before the first check.
    pub fn connect_auto(&self, gid: i64) {
        let mut profiles = match self.store.borrow().profiles(Some(gid)) {
            Ok(profiles) => profiles,
            Err(_) => return,
        };
        profiles.sort_by_key(|p| match p.latency {
            0 => i32::MAX - 1,
            latency if latency < 0 => i32::MAX,
            latency => latency,
        });
        self.engine.send(Command::ConnectAuto {
            profiles,
            gid,
            settings: Box::new(self.settings.borrow().clone()),
        });
    }

    pub fn disconnect(&self) {
        self.engine.send(Command::Disconnect);
    }

    /// After an import or subscription, opening an empty group is pointless —
    /// move the view to where servers exist.
    pub fn focus_group_with_servers(&self) {
        let store = self.store.borrow();
        if store
            .profiles(Some(self.current_gid.get()))
            .map(|p| !p.is_empty())
            .unwrap_or(false)
        {
            return;
        }
        let gid = first_non_empty_group(&store);
        drop(store);
        self.current_gid.set(gid);
    }
}

/// Group with which the list opens: the one containing the last selected server,
/// otherwise the first non-empty group, otherwise the default group.
fn starting_group(store: &Store, selected_profile: i64) -> i64 {
    if selected_profile != 0 {
        if let Ok(Some(profile)) = store.profile(selected_profile) {
            return profile.gid;
        }
    }
    first_non_empty_group(store)
}

fn first_non_empty_group(store: &Store) -> i64 {
    store
        .groups()
        .unwrap_or_default()
        .into_iter()
        .find(|group| {
            store
                .profiles(Some(group.id))
                .map(|p| !p.is_empty())
                .unwrap_or(false)
        })
        .map(|group| group.id)
        .unwrap_or(1)
}
