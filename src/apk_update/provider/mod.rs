use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::Unreachable;

pub mod mirror;

pub use mirror::ApkPure;

#[derive(Debug, Default)]
pub struct Cancel(AtomicBool);

impl Cancel {
    pub fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> Result<(), Unreachable> {
        if self.stopped() {
            Err(Unreachable::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub name: String,
    pub code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archives {
    pub base: PathBuf,
    pub split: PathBuf,
}

impl Archives {
    pub fn distinct(&self) -> Vec<&Path> {
        if self.base == self.split {
            vec![self.base.as_path()]
        } else {
            vec![self.base.as_path(), self.split.as_path()]
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Asking {
        provider: &'static str,
    },
    Fetching {
        file: String,
        done: u64,
        total: Option<u64>,
    },
    Verifying {
        file: String,
    },
}

pub trait Provider {
    fn name(&self) -> &'static str;
    fn newest(&self, progress: &mut dyn FnMut(Progress)) -> Result<Available, Unreachable>;
    fn fetch(
        &self,
        version: &Available,
        cancel: &Cancel,
        into: &Path,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Archives, Unreachable>;
}
