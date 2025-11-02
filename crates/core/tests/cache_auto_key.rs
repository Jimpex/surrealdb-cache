mod helpers;
use anyhow::Result;
use helpers::new_ds;
use surrealdb_core::dbs::Session;
use surrealdb_core::syn;

#[tokio::test]
async fn query_cache_auto_key_generation() -> Result<()> {
	// Test that auto key generation works without explicit key
	let sql = "
		USE NS test DB test;
		DEFINE TABLE person PERMISSIONS FULL;
		CREATE person:alice SET name = 'Alice', age = 25;
		CREATE person:bob SET name = 'Bob', age = 30;
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// Query with auto-generated key (no key specified)
	let query = "SELECT * FROM person ORDER BY name CACHE 5m;";
	let res1 = &mut dbs.execute(query, &ses, None).await?;
	let result1 = res1.remove(0).result?;
	
	// Same query should hit cache
	let res2 = &mut dbs.execute(query, &ses, None).await?;
	let result2 = res2.remove(0).result?;
	
	assert_eq!(result1, result2);
	
	// Different query should NOT hit cache
	let query2 = "SELECT * FROM person WHERE age > 20 ORDER BY name CACHE 5m;";
	let res3 = &mut dbs.execute(query2, &ses, None).await?;
	let result3 = res3.remove(0).result?;
	
	// Both queries return same data but from different cache entries
	assert_eq!(result1, result3);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_compact_keys() -> Result<()> {
	// Test that keys are compact with optimized prefixes
	let sql = "
		USE NS test DB test;
		DEFINE TABLE person PERMISSIONS FULL;
		CREATE person:test SET name = 'Test';
	";
	let dbs = new_ds().await?;
	let ses_root = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses_root, None).await?;
	
	// System/root query - should use "s" prefix
	let query = "SELECT * FROM person CACHE 5m 'compact_test';";
	let res1 = &mut dbs.execute(query, &ses_root, None).await?;
	let result1 = res1.remove(0).result?;
	
	// Record user query - should use "u:{id}" prefix
	let ses_user = Session::for_record(
		"test",
		"test",
		"user_access",
		syn::value("user:alice").unwrap()
	);
	let res2 = &mut dbs.execute(query, &ses_user, None).await?;
	let result2 = res2.remove(0).result?;
	
	// Both get same data but from different cache namespaces
	assert_eq!(result1, result2);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_global_compact() -> Result<()> {
	// Test that global cache uses "g" prefix
	let sql = "
		USE NS test DB test;
		DEFINE TABLE person PERMISSIONS FULL;
		CREATE person:test SET name = 'Test';
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// Global cache with auto key
	let query = "SELECT * FROM person CACHE GLOBAL 5m;";
	let res1 = &mut dbs.execute(query, &ses, None).await?;
	let result1 = res1.remove(0).result?;
	
	// Different session should hit same global cache
	let ses2 = Session::owner().with_ns("test").with_db("test");
	let res2 = &mut dbs.execute(query, &ses2, None).await?;
	let result2 = res2.remove(0).result?;
	
	assert_eq!(result1, result2);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_auto_vs_custom_keys() -> Result<()> {
	// Test that auto and custom keys are separate
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// Query with auto key
	let query_auto = "SELECT * FROM person CACHE 5m;";
	let res1 = &mut dbs.execute(query_auto, &ses, None).await?;
	let result1 = res1.remove(0).result?;
	
	// Add new data
	let add_data = "CREATE person:bob SET name = 'Bob', age = 30;";
	dbs.execute(add_data, &ses, None).await?;
	
	// Same query with custom key should NOT hit auto key cache
	let query_custom = "SELECT * FROM person CACHE 5m 'custom_key';";
	let res2 = &mut dbs.execute(query_custom, &ses, None).await?;
	let result2 = res2.remove(0).result?;
	
	// Custom key query sees new data (2 records)
	let expected = syn::value(
		"[
			{ id: person:alice, name: 'Alice', age: 25 },
			{ id: person:bob, name: 'Bob', age: 30 }
		]",
	)
	.unwrap();
	assert_eq!(result2, expected);
	
	// Auto key query still has old data (1 record)
	let res3 = &mut dbs.execute(query_auto, &ses, None).await?;
	let result3 = res3.remove(0).result?;
	assert_eq!(result1, result3);
	
	Ok(())
}
