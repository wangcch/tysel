//! Live Redis test. Skipped unless `TYSEL_REDIS_TEST_URL` is set.

use tysel_engine::Value;

#[tokio::test]
async fn redis_roundtrips_bounded_string_operations() {
    let Some(url) = std::env::var("TYSEL_REDIS_TEST_URL").ok().filter(|value| !value.is_empty())
    else {
        assert_ne!(
            std::env::var("TYSEL_REDIS_TEST_REQUIRED").as_deref(),
            Ok("1"),
            "TYSEL_REDIS_TEST_URL is required for this test run"
        );
        eprintln!("skipping live Redis test (set TYSEL_REDIS_TEST_URL)");
        return;
    };
    tysel_cap_redis::configure(Some(url.clone()), false);
    let key = format!("tysel:test:{}", std::process::id());

    tysel_cap_redis::del(std::slice::from_ref(&key)).await.unwrap();
    assert_eq!(tysel_cap_redis::get(&key).await.unwrap(), Value::Null);
    tysel_cap_redis::set(&key, "hello", Some(60)).await.unwrap();
    assert_eq!(tysel_cap_redis::get(&key).await.unwrap(), Value::String("hello".into()));
    assert_eq!(tysel_cap_redis::exists(&key).await.unwrap(), Value::Bool(true));
    assert_eq!(tysel_cap_redis::expire(&key, 30).await.unwrap(), Value::Bool(true));
    tysel_cap_redis::set(&key, "", Some(60)).await.unwrap();
    assert_eq!(tysel_cap_redis::get(&key).await.unwrap(), Value::String(String::new()));

    let client = redis::Client::open(url).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("SET")
        .arg(&key)
        .arg(vec![b'x'; 1_048_577])
        .query_async::<String>(&mut connection)
        .await
        .unwrap();
    assert!(tysel_cap_redis::get(&key).await.unwrap_err().to_string().contains("exceeded"));
    assert_eq!(tysel_cap_redis::del(std::slice::from_ref(&key)).await.unwrap(), Value::Number(1.0));
}
