use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::dbs::Options;

/// Generate auto cache key from SELECT statement
/// Hashes the entire serialized statement for deterministic keys
/// 
/// Returns: "a{base62_hash}" (e.g., "a3kTyX9mPqR1")
pub fn generate_auto_cache_key(statement: &str) -> String {
	// Hash the entire statement string (already normalized via Display)
	// Parameters are already embedded in the statement string
	let hash = hash_query_components(statement, &[]);
	
	format!("a{}", hash) // "a" prefix for auto-generated
}

/// Generate a short, deterministic hash from query components
/// This produces compact keys while maintaining collision resistance
/// 
/// Key format: base62(hash) - typically 11 chars for 64-bit hash
/// Example: "3kTyX9mPqR1"
fn hash_query_components(statement: &str, params: &[(String, String)]) -> String {
	let mut hasher = DefaultHasher::new();
	
	// Hash the statement (already normalized/serialized)
	statement.hash(&mut hasher);
	
	// Hash parameters in sorted order for determinism
	for (key, value) in params {
		key.hash(&mut hasher);
		value.hash(&mut hasher);
	}
	
	let hash = hasher.finish();
	
	// Convert to base62 for shorter representation
	// base62 = [0-9a-zA-Z] = 62 chars
	// 64-bit hash in base62 = ~11 chars vs 16 hex chars
	base62_encode(hash)
}

/// Encode u64 as base62 string for compact representation
fn base62_encode(mut num: u64) -> String {
	const CHARSET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
	
	if num == 0 {
		return "0".to_string();
	}
	
	let mut result = Vec::new();
	while num > 0 {
		result.push(CHARSET[(num % 62) as usize]);
		num /= 62;
	}
	result.reverse();
	String::from_utf8(result).unwrap()
}

/// Build full cache key with namespace prefix
/// 
/// Key structure: {auth}::{ns}::{db}::{query_key}
/// - auth: "g" (global), "s" (system), "u:{id}" (user), "n" (unauthenticated)
/// - ns: namespace name
/// - db: database name
/// - query_key: user-provided or auto-generated hash
/// 
/// Examples:
/// - "g::test::main::mykey" - global cache
/// - "u:alice::test::main::a3kTyX9mPqR" - user-scoped with auto key
/// - "s::test::main::reports" - system-scoped with custom key
/// - "n::test::main::aF7nK2pQx" - unauthenticated with auto key
pub fn build_cache_key(query_key: &str, opt: &Options, global: bool) -> String {
	use crate::iam::Level;
	
	// Auth scope: compact single-char prefixes
	let auth_scope = if global {
		"g".to_string()
	} else {
		match opt.auth.level() {
			Level::No => "n".to_string(),                       // unauthenticated
			Level::Root => "s".to_string(),                     // system/root
			Level::Namespace(_) => "s".to_string(),             // namespace level
			Level::Database(_, _) => "s".to_string(),           // database level
			Level::Record(_, _, _) => {
				// Record user: use auth ID (e.g., "u:alice")
				let id = opt.auth.id();
				if id.starts_with("user:") || id.starts_with("record:") {
					format!("u:{}", id.split(':').nth(1).unwrap_or(id))
				} else {
					format!("u:{}", id)
				}
			}
		}
	};
	
	// Use namespace/database names
	let ns = opt.ns().unwrap_or("*");
	let db = opt.db().unwrap_or("*");
	
	format!("{}::{}::{}::{}", auth_scope, ns, db, query_key)
}
