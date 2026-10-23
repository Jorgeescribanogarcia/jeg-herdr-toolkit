//! The agent states reported by herdr.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Working,
    Blocked,
    Done,
    Idle,
    Unknown,
}

impl State {
    /// Each `IconSet` lists its glyphs in this same order.
    pub const ALL: [State; 5] = [State::Working, State::Blocked, State::Done, State::Idle, State::Unknown];

    /// herdr's `agent_status`: `working`, `blocked`, `done`, `idle`; anything else is unknown.
    pub fn from_herdr(status: &str) -> State {
        match status {
            "working" => State::Working,
            "blocked" => State::Blocked,
            "done" => State::Done,
            "idle" => State::Idle,
            _ => State::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            State::Working => "Working",
            State::Blocked => "Waiting for you",
            State::Done => "Done",
            State::Idle => "Idle",
            State::Unknown => "Unknown",
        }
    }

    pub fn index(self) -> usize {
        State::ALL.iter().position(|&s| s == self).expect("ALL contains every state")
    }
}
