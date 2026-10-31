use axum::response::IntoResponse;
use doggy_dns_console::api::error::ApiError;
use doggy_dns_console::error::DataError;
use http_body_util::BodyExt;

#[tokio::test]
async fn rate_limited_error_maps_to_429() {
    let response = ApiError(DataError::RateLimited("too many requests".into())).into_response();
    assert_eq!(response.status(), 429);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"]["code"], "rate_limited");
}
