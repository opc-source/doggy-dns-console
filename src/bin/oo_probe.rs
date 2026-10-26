use anyhow::Result;
use doggy_dns_console::config::Config;
use doggy_dns_console::datasource::openobserve::OpenObserveClient;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env()?;
    let client = OpenObserveClient::new(config.openobserve.clone())?;

    let streams = client.list_streams().await?;
    println!("streams ({}):", streams.len());
    for stream in &streams {
        println!("  - {stream}");
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let sql = format!(
        "SELECT * FROM \"{}\" ORDER BY _timestamp DESC LIMIT 5",
        config.openobserve.log_stream
    );
    match client.search_logs(&sql, now - 60 * 60 * 1000, now, 5).await {
        Ok(rows) => {
            println!("recent log rows ({}):", rows.len());
            for row in rows {
                println!("{}", serde_json::to_string_pretty(&row)?);
            }
        }
        Err(error) => println!("log probe failed: {error}"),
    }

    let promql = format!("sum(increase({}[5m]))", config.openobserve.metric_queries);
    match client.query_promql(&promql, now - 300_000, now, 60).await {
        Ok(series) => {
            println!("metric probe series ({}):", series.len());
            for item in series {
                println!("labels={:?} points={:?}", item.labels, item.points);
            }
        }
        Err(error) => println!("metric probe failed: {error}"),
    }

    Ok(())
}
