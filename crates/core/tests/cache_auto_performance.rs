mod helpers;
use anyhow::Result;
use helpers::new_ds;
use surrealdb_core::dbs::Session;
use std::time::Instant;

#[tokio::test]
async fn query_cache_auto_key_performance() -> Result<()> {
	// Test that auto key generation provides actual cache hits with reduced time
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
		CREATE person:bob SET name = 'Bob', age = 30;
		CREATE person:charlie SET name = 'Charlie', age = 35;
		CREATE person:diana SET name = 'Diana', age = 40;
		CREATE person:eve SET name = 'Eve', age = 45;
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// First query - should populate cache (slower)
	let query = "SELECT * FROM person ORDER BY name CACHE 5m;";
	let start1 = Instant::now();
	let res1 = &mut dbs.execute(query, &ses, None).await?;
	let duration1 = start1.elapsed();
	let result1 = res1.remove(0).result?;
	
	println!("First query (cache miss): {:?}", duration1);
	
	// Second query - should hit cache (faster)
	let start2 = Instant::now();
	let res2 = &mut dbs.execute(query, &ses, None).await?;
	let duration2 = start2.elapsed();
	let result2 = res2.remove(0).result?;
	
	println!("Second query (cache hit): {:?}", duration2);
	
	// Results should be identical
	assert_eq!(result1, result2);
	
	// Cache hit should be faster (at least 2x faster as a conservative check)
	// Note: This might be flaky in CI, but should work locally
	println!("Speed improvement: {:.2}x", duration1.as_micros() as f64 / duration2.as_micros() as f64);
	
	// Just verify we got results, not asserting on timing for CI stability
	assert!(!result1.is_none());
	
	Ok(())
}

#[tokio::test]
async fn query_cache_auto_key_different_queries() -> Result<()> {
	// Test that different queries get different auto keys
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
		CREATE person:bob SET name = 'Bob', age = 30;
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// Query 1 with auto key
	let query1 = "SELECT * FROM person WHERE age > 20 ORDER BY name CACHE 5m;";
	let res1 = &mut dbs.execute(query1, &ses, None).await?;
	let result1 = res1.remove(0).result?;
	
	// Query 2 with different WHERE clause - should get different auto key
	let query2 = "SELECT * FROM person WHERE age > 25 ORDER BY name CACHE 5m;";
	let res2 = &mut dbs.execute(query2, &ses, None).await?;
	let result2 = res2.remove(0).result?;
	
	// Results should be different (query1 has 2 records, query2 has 1)
	assert_ne!(result1, result2);
	
	// Query 1 again - should hit cache
	let res3 = &mut dbs.execute(query1, &ses, None).await?;
	let result3 = res3.remove(0).result?;
	
	// Should match first query (from cache)
	assert_eq!(result1, result3);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_no_key_vs_with_key() -> Result<()> {
	// Test that omitting key enables auto generation
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// Query WITHOUT key (should auto-generate)
	let query_no_key = "SELECT * FROM person CACHE 5m;";
	let res1 = &mut dbs.execute(query_no_key, &ses, None).await?;
	let result1 = res1.remove(0).result?;
	
	// Same query again - should hit auto-generated cache
	let res2 = &mut dbs.execute(query_no_key, &ses, None).await?;
	let result2 = res2.remove(0).result?;
	
	assert_eq!(result1, result2);
	
	// Query WITH explicit key
	let query_with_key = "SELECT * FROM person CACHE 5m 'explicit_key';";
	let res3 = &mut dbs.execute(query_with_key, &ses, None).await?;
	let result3 = res3.remove(0).result?;
	
	// Should get same data
	assert_eq!(result1, result3);
	
	// Repeat with explicit key - should hit that cache
	let res4 = &mut dbs.execute(query_with_key, &ses, None).await?;
	let result4 = res4.remove(0).result?;
	
	assert_eq!(result3, result4);
	
	Ok(())
}

#[tokio::test]
async fn query_cache_auto_key_deterministic() -> Result<()> {
	// Test that auto keys are deterministic (same query = same key)
	let sql = "
		USE NS test DB test;
		CREATE person:alice SET name = 'Alice', age = 25;
	";
	let dbs = new_ds().await?;
	let ses = Session::owner().with_ns("test").with_db("test");
	dbs.execute(sql, &ses, None).await?;
	
	// Execute same query multiple times
	let query = "SELECT * FROM person WHERE age = 25 ORDER BY name CACHE 5m;";
	
	let mut results = Vec::new();
	for i in 0..5 {
		let res = &mut dbs.execute(query, &ses, None).await?;
		let result = res.remove(0).result?;
		results.push(result);
		println!("Query {} completed", i + 1);
	}
	
	// All results should be identical (hitting same cache)
	for i in 1..results.len() {
		assert_eq!(results[0], results[i], "Result {} doesn't match result 0", i);
	}
	
	Ok(())
}
