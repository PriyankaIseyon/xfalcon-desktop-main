//! Two small keepsakes the interface keeps between launches: the day the
//! greeting last showed, and how many messages this installation has sent.
//!
//! One JSON document at `<app_local_data>/keepsakes.json`. Like the appearance
//! preferences these describe this installation, not the account, and the
//! renderer cannot keep them itself: `src/security/frontend-boundary` forbids
//! browser storage in production frontend sources.
//!
//! Reading never fails. A missing, empty or malformed file reads as "nothing
//! kept yet", and the next write repairs it. These are conveniences, so the
//! worst a lost file does is greet once more or count again from zero.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::settings::write_atomically;

const KEEPSAKES_FILE: &str = "keepsakes.json";

/// Both operations read the document, change it and write it back. The commands
/// run on separate threads, so without this two of them could each read the old
/// document and the later write would drop the earlier one's change.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Deserialize, Serialize)]
struct Keepsakes {
    /// The local calendar day, `YYYY-MM-DD`, the greeting last showed.
    #[serde(default)]
    greeted_on: Option<String>,
    #[serde(default)]
    messages_sent: u64,
}

/// A `YYYY-MM-DD` day. Only the shape is checked: the renderer owns the local
/// calendar, and the store only needs to tell one day from another.
fn is_day(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(at, byte)| {
            if at == 4 || at == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

pub struct KeepsakeStore {
    path: PathBuf,
}

impl KeepsakeStore {
    #[must_use]
    pub fn new(app_local_data: PathBuf) -> Self {
        Self {
            path: app_local_data.join(KEEPSAKES_FILE),
        }
    }

    fn read(&self) -> Keepsakes {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write(&self, keepsakes: &Keepsakes) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(keepsakes).map_err(io::Error::other)?;
        write_atomically(&self.path, &bytes)
    }

    /// Whether `day` is the first the greeting is claimed for. Claiming records
    /// the day, so the same day answers `false` from then on.
    ///
    /// The day is recorded before the answer is returned: a write that fails
    /// is an error, not a `true` the next launch would hand out again.
    pub fn claim_greeting(&self, day: &str) -> io::Result<bool> {
        if !is_day(day) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a YYYY-MM-DD day",
            ));
        }
        let _guard = WRITE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut keepsakes = self.read();
        if keepsakes.greeted_on.as_deref() == Some(day) {
            return Ok(false);
        }
        keepsakes.greeted_on = Some(day.to_owned());
        self.write(&keepsakes)?;
        Ok(true)
    }

    /// Counts one message sent and returns the total, this one included.
    pub fn count_message(&self) -> io::Result<u64> {
        let _guard = WRITE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut keepsakes = self.read();
        keepsakes.messages_sent = keepsakes.messages_sent.saturating_add(1);
        self.write(&keepsakes)?;
        Ok(keepsakes.messages_sent)
    }
}

pub mod commands;

#[cfg(test)]
mod tests;
