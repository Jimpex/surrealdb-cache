use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Result, ensure};
use reblessive::tree::Stk;
use dashmap::DashMap;
use std::sync::OnceLock;

use crate::ctx::Context;
use crate::dbs::{Iterator, Options, Statement};
use crate::doc::CursorDoc;
use crate::err::Error;
use crate::expr::order::Ordering;
use crate::expr::{
	Cache, Cond, Explain, Expr, Fetchs, Fields, FlowResultExt as _, Groups, Limit, Splits, Start,
	Timeout, With,
};
use crate::fmt::Fmt;
use crate::idx::planner::{QueryPlanner, RecordStrategy, StatementContext};
use crate::val::{Datetime, Value};

const TARGET: &str = "surrealdb::core::expr::statements::select";

// Global query cache - single static instance shared across all queries
pub static QUERY_CACHE: OnceLock<DashMap<String, (SystemTime, Value)>> = OnceLock::new();

/// Clean up expired entries from the query cache
/// Returns the number of entries removed
pub fn cleanup_query_cache() -> usize {
	if let Some(cache) = QUERY_CACHE.get() {
		let before_count = cache.len();
		cache.retain(|_, (exp, _)| SystemTime::now() < *exp);
		let after_count = cache.len();
		before_count - after_count
	} else {
		0
	}
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct SelectStatement {
	/// The foo,bar part in SELECT foo,bar FROM baz.
	pub expr: Fields,
	pub omit: Vec<Expr>,
	pub only: bool,
	/// The baz part in SELECT foo,bar FROM baz.
	pub what: Vec<Expr>,
	pub with: Option<With>,
	pub cond: Option<Cond>,
	pub split: Option<Splits>,
	pub group: Option<Groups>,
	pub order: Option<Ordering>,
	pub limit: Option<Limit>,
	pub start: Option<Start>,
	pub fetch: Option<Fetchs>,
	pub version: Option<Expr>,
	pub cache: Option<Cache>,
	pub timeout: Option<Timeout>,
	pub parallel: bool,
	pub explain: Option<Explain>,
	pub tempfiles: bool,
}

impl Default for SelectStatement {
	fn default() -> Self {
		SelectStatement {
			expr: Fields::all(),
			omit: vec![],
			only: false,
			what: Vec::new(),
			with: None,
			cond: None,
			split: None,
			group: None,
			order: None,
			limit: None,
			start: None,
			fetch: None,
			version: None,
			cache: None,
			timeout: None,
			parallel: false,
			explain: None,
			tempfiles: false,
		}
	}
}

impl SelectStatement {
	/// Check if computing this type can be done on a read only transaction.
	pub(crate) fn read_only(&self) -> bool {
		self.expr.read_only()
			&& self.what.iter().all(|v| v.read_only())
			&& self.cond.as_ref().map(|x| x.0.read_only()).unwrap_or(true)
	}

	/// Process this type returning a computed simple Value
	pub(crate) async fn compute(
		&self,
		stk: &mut Stk,
		ctx: &Context,
		opt: &Options,
		doc: Option<&CursorDoc>,
	) -> Result<Value> {
		// MEMORY cache with auto key generation support
		if let Some(cache_cfg) = &self.cache {
			if matches!(cache_cfg.mode, crate::expr::CacheMode::Memory) {
				// Generate query key: custom or auto-generated
				let query_key = if let Some(custom_key) = &cache_cfg.key {
					custom_key.clone()
				} else {
					// Auto-generate key from statement
					crate::dbs::cache::generate_auto_cache_key(&self.to_string())
				};

				// Build full cache key with auth/ns/db prefix
				let cache_key =
					crate::dbs::cache::build_cache_key(&query_key, &*opt, cache_cfg.global);

				let map = QUERY_CACHE.get_or_init(|| DashMap::new());

				trace!(target: TARGET, cache_key = %cache_key, query_key = %query_key, global = cache_cfg.global, "Checking cache");

				if let Some(entry) = map.get(&cache_key) {
					let (exp, val) = entry.value();
					let now = SystemTime::now();

					if now < *exp {
						trace!(target: TARGET, cache_key = %cache_key, "Cache hit");
						return Ok(val.clone());
					} else {
						trace!(target: TARGET, cache_key = %cache_key, "Cache expired");
					}
				} else {
					trace!(target: TARGET, cache_key = %cache_key, "Cache miss");
				}
			}
		}

		// Valid options?
		opt.valid_for_db()?;
		// Assign the statement
		let stm = Statement::from_select(stk, ctx, opt, doc, self).await?;
		// Create a new iterator
		let mut i = Iterator::new();
		// Ensure futures are stored and the version is set if specified

		let version = match &self.version {
			Some(v) => Some(
				stk.run(|stk| v.compute(stk, ctx, opt, doc))
					.await
					.catch_return()?
					.cast_to::<Datetime>()?
					.to_version_stamp()?,
			),
			_ => None,
		};
		let opt = Arc::new(opt.clone().with_version(version));

		// Extract the limits
		i.setup_limit(stk, ctx, &opt, &stm).await?;
		// Fail for multiple targets without a limit
		ensure!(
			!self.only || i.is_limit_one_or_zero() || self.what.len() <= 1,
			Error::SingleOnlyOutput
		);
		// Check if there is a timeout
		let ctx = stm.setup_timeout(stk, ctx, &opt, doc).await?;

		// Get a query planner
		let mut planner = QueryPlanner::new();

		let stm_ctx = StatementContext::new(&ctx, &opt, &stm)?;
		// Loop over the select targets
		for w in self.what.iter() {
			i.prepare(stk, &ctx, &opt, doc, &mut planner, &stm_ctx, w).await?;
		}
		// Attach the query planner to the context
		let ctx = stm.setup_query_planner(planner, ctx);

		// Process the statement
		let res = i.output(stk, &ctx, &opt, &stm, RecordStrategy::KeysAndValues).await?;
		// Catch statement timeout
		ensure!(!ctx.is_timedout().await?, Error::QueryTimedout);

		let out = if self.only {
			match res {
				Value::Array(mut array) => {
					if array.is_empty() {
						Value::None
					} else {
						ensure!(array.len() == 1, Error::SingleOnlyOutput);
						array.0.pop().expect("array has exactly one element")
					}
				}
				x => x,
			}
		} else {
			res
		};

		// Store into cache if configured
		if let Some(cache_cfg) = &self.cache {
			if matches!(cache_cfg.mode, crate::expr::CacheMode::Memory) {
				// Generate query key: custom or auto-generated
				let query_key = if let Some(custom_key) = &cache_cfg.key {
					custom_key.clone()
				} else {
					// Auto-generate key from statement
					crate::dbs::cache::generate_auto_cache_key(&self.to_string())
				};

				// Build full cache key with auth/ns/db prefix
				let cache_key =
					crate::dbs::cache::build_cache_key(&query_key, &*opt, cache_cfg.global);

				// Compute expiration time using the expr::Cache method
				let expiration = cache_cfg
					.compute_expiration(stk, &ctx, &*opt, doc)
					.await?;
				
				let map = QUERY_CACHE.get_or_init(|| DashMap::new());
				map.insert(cache_key.clone(), (expiration, out.clone()));
				trace!(target: TARGET, cache_key = %cache_key, total_entries = map.len(), "Stored result in cache");
			}
		}

		Ok(out)
	}
}

impl fmt::Display for SelectStatement {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "SELECT {}", self.expr)?;
		if !self.omit.is_empty() {
			write!(f, " OMIT {}", Fmt::comma_separated(self.omit.iter()))?
		}
		write!(f, " FROM")?;
		if self.only {
			f.write_str(" ONLY")?
		}
		write!(f, " {}", Fmt::comma_separated(self.what.iter()))?;
		if let Some(ref v) = self.with {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.cond {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.split {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.group {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.order {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.limit {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.start {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.fetch {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.version {
			write!(f, " VERSION {v}")?
		}
		if let Some(ref v) = self.cache {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.timeout {
			write!(f, " {v}")?
		}
		if self.parallel {
			f.write_str(" PARALLEL")?
		}
		if let Some(ref v) = self.explain {
			write!(f, " {v}")?
		}
		Ok(())
	}
}
