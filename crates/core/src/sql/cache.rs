use std::fmt;

use crate::sql::Expr;

/// Cache mode for query results
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
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

/// Cache configuration for SELECT statements
/// Stores the expiration as an unevaluated expression (Duration or Datetime)
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
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

use std::hash::{Hash, Hasher};

impl Hash for Cache {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.mode.hash(state);
        // Hash a deterministic string representation of the expression
        // Expr implements Display; use that to avoid requiring Expr: Hash
        use core::fmt::Write as _;
        let mut s = String::new();
        let _ = write!(&mut s, "{}", self.expiration);
        s.hash(state);
        self.key.hash(state);
        self.global.hash(state);
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

impl From<Cache> for crate::expr::Cache {
	fn from(v: Cache) -> Self {
		Self {
			mode: match v.mode {
				CacheMode::Memory => crate::expr::CacheMode::Memory,
				CacheMode::Disk => crate::expr::CacheMode::Disk,
			},
			expiration: v.expiration.into(),
			key: v.key,
			global: v.global,
		}
	}
}

impl From<crate::expr::Cache> for Cache {
	fn from(v: crate::expr::Cache) -> Self {
		Self {
			mode: match v.mode {
				crate::expr::CacheMode::Memory => CacheMode::Memory,
				crate::expr::CacheMode::Disk => CacheMode::Disk,
			},
			expiration: v.expiration.into(),
			key: v.key,
			global: v.global,
		}
	}
}
