mod helpers;

use anyhow::Result;
use helpers::new_ds;
use surrealdb_core::dbs::Session;
use surrealdb_core::syn;

#[tokio::test]
async fn query_cache_auth_scoped_root() -> Result<()> {
	// Test that root-level auth gets scoped cache keys
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
		CREATE person:bob SET name = 'Bob', age = 30;
		SELECT * FROM person ORDER BY name CACHE 5m 'root_cache_key';
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	let res = &mut dbs.execute(sql, &ses, None).await?;
	assert_eq!(res.len(), 4);
	
	res.remove(0).result?; // USE
	res.remove(0).result?; // CREATE alice
	res.remove(0).result?; // CREATE bob
	
	// First query should populate cache
	let tmp = res.remove(0).result?;
	let val = syn::value(
		"[
			{ id: person:alice, name: 'Alice', age: 25 },
			{ id: person:bob, name: 'Bob', age: 30 }
		]",
	)
	.unwrap();
	assert_eq!(tmp, val);
	
	// Second query with same session should hit cache
	let sql2 = "SELECT * FROM person ORDER BY name CACHE 5m 'root_cache_key';";
	let res2 = &mut dbs.execute(sql2, &ses, None).await?;
	let tmp2 = res2.remove(0).result?;
	assert_eq!(tmp2, val);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_auth_scoped_different_sessions() -> Result<()> {
	// Test that different auth sessions get separate cache entries
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
		SELECT * FROM person CACHE 5m 'session_test';
	";
	let dbs = new_ds().await?;
	
	// Root session
	let ses_root = Session::owner().with_ns("test").with_db("test");
	let res_root = &mut dbs.execute(sql, &ses_root, None).await?;
	res_root.remove(0).result?; // USE
	res_root.remove(0).result?; // CREATE
	let result_root = res_root.remove(0).result?;
	
	// Record-based session
	let ses_user = Session::for_record(
		"test",
		"test",
		"user_access",
		syn::value("user:alice").unwrap()
	);
	let res_user = &mut dbs.execute(sql, &ses_user, None).await?;
	res_user.remove(0).result?; // USE
	// Skip CREATE since it already exists
	res_user.remove(0).result?; // SELECT
	let result_user = res_user.remove(0).result?;
	
	// Both should get same data but from different cache entries
	assert_eq!(result_root, result_user);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_global_shared() -> Result<()> {
	// Test that GLOBAL cache is shared across auth sessions
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
		SELECT * FROM person CACHE GLOBAL 5m 'global_test';
	";
	let dbs = new_ds().await?;
	
	// Root session
	let ses_root = Session::owner().with_ns("test").with_db("test");
	let res_root = &mut dbs.execute(sql, &ses_root, None).await?;
	res_root.remove(0).result?; // USE
	res_root.remove(0).result?; // CREATE
	let result_root = res_root.remove(0).result?;
	
	// Record-based session - should hit the same global cache
	let ses_user = Session::for_record(
		"test",
		"test",
		"user_access",
		syn::value("user:bob").unwrap()
	);
	let res_user = &mut dbs.execute("SELECT * FROM person CACHE GLOBAL 5m 'global_test';", &ses_user, None).await?;
	let result_user = res_user.remove(0).result?;
	
	// Should get same data from global cache
	assert_eq!(result_root, result_user);
	
	Ok(())
}
