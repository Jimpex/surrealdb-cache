use std::fmt;
use std::time::SystemTime;

use anyhow::Result;
use reblessive::tree::Stk;

use crate::ctx::Context;
use crate::dbs::Options;
use crate::doc::CursorDoc;
use crate::expr::{Expr, FlowResultExt as _};
use crate::val::Value;

/// Cache mode for query results
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd)]
#[non_exhaustive]
pub enum CacheMode {
	/// In-memory cache (volatile, fast)
	Memory,
	/// Disk-based cache (persistent, slower)
	Disk,
}

impl Default for CacheMode {
	fn default() -> Self {
		Self::Memory
	}
}

impl fmt::Display for CacheMode {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			Self::Memory => f.write_str("MEMORY"),
			Self::Disk => f.write_str("DISK"),
		}
	}
}

/// Cache configuration for SELECT statements (execution layer)
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub struct Cache {
	/// Cache mode (MEMORY or DISK)
	pub mode: CacheMode,
	/// Expression that evaluates to Duration or Datetime for expiration
	pub expiration: Expr,
	/// Optional custom cache key
	pub key: Option<String>,
	/// Whether the cache is global (not scoped to auth)
	pub global: bool,
}

impl Cache {
	/// Compute the expiration time for the cache entry
	/// Evaluates the expiration expression and returns SystemTime
	pub(crate) async fn compute_expiration(
		&self,
		stk: &mut Stk,
		ctx: &Context,
		opt: &Options,
		doc: Option<&CursorDoc>,
	) -> Result<SystemTime> {
		let v = stk.run(|stk| self.expiration.compute(stk, ctx, opt, doc)).await.catch_return()?;
		
		match v {
			Value::Duration(d) => {
				let std_dur = std::time::Duration::from(d);
				Ok(SystemTime::now() + std_dur)
			}
			Value::Datetime(dt) => {
				let timestamp = dt.timestamp();
				let std_dur = std::time::Duration::from_secs(timestamp as u64);
				Ok(SystemTime::UNIX_EPOCH + std_dur)
			}
			_ => Ok(SystemTime::now()),
		}
	}
}

impl fmt::Display for Cache {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.write_str("CACHE")?;
		// Only show mode if it's not the default (MEMORY)
		if matches!(self.mode, CacheMode::Disk) {
			write!(f, " {}", self.mode)?;
		}
		if self.global {
			f.write_str(" GLOBAL")?;
		}
		write!(f, " {}", self.expiration)?;
		if let Some(ref key) = self.key {
			write!(f, " \"{}\"", key)?;
		}
		Ok(())
	}
}
