//! A flag a long search polls so a newer request can stop it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Returned by `Cancel::check` once the flag is set.
#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

/// A shared flag: clones see the same state.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_set() { Err(Cancelled) } else { Ok(()) }
    }
}
