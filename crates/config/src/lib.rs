//! Profiles, links, and sing-box configuration generation.

pub mod generate;
pub mod link;
pub mod profile;
pub mod route;
pub mod settings;
pub mod share;
pub mod subscription;
pub mod xray;

pub use generate::{generate, generate_test, GeneratedConfig};
pub use profile::{Group, Profile};
pub use settings::Settings;
pub use subscription::Parsed;
