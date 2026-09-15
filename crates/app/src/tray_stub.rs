//! Tray fallback for platforms where StatusNotifierItem is unavailable.

/// What the user wants after clicking the icon.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum Action {
    Show,
    Quit,
}

/// Non-Linux builds currently keep the window open instead of hiding it into
/// a tray implementation that does not exist on the platform.
#[derive(Clone, Copy)]
pub struct Tray;

impl Tray {
    pub fn is_alive(&self) -> bool {
        false
    }

    pub fn set_connected(&self, _name: Option<String>) {}
}

pub fn start(_actions: async_channel::Sender<Action>) -> Tray {
    Tray
}
