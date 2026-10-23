//! What the viewer, the agent status icons and the settings panel share: the `settings.toml`
//! format, herdr's agent states and the icon sets.

pub mod fs_util;
pub mod icon_sets;
pub mod publish;
pub mod settings;
pub mod state;

pub use icon_sets::{ICON_SETS, IconSet};
pub use settings::Settings;
pub use state::State;
