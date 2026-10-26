use adk_rust::session::{CreateRequest, ListRequest, SessionService, SqliteSessionService};

#[tokio::test]
async fn sqlite_session_service_migrates_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("console.db");
    std::fs::File::create(&path).unwrap();
    let url = format!("sqlite:{}", path.display());
    let service = SqliteSessionService::new(&url).await.unwrap();
    service.migrate().await.unwrap();

    let session = service
        .create(CreateRequest {
            app_name: "doggy-dns-console".to_string(),
            user_id: "local".to_string(),
            session_id: Some("s1".to_string()),
            state: Default::default(),
        })
        .await
        .unwrap();
    assert_eq!(session.id(), "s1");

    let sessions = service
        .list(ListRequest {
            app_name: "doggy-dns-console".to_string(),
            user_id: "local".to_string(),
            limit: None,
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
}
