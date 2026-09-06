use super::*;

#[tokio::test]
async fn file_save_requires_authenticated_bounded_content_precondition() {
    let _guard = api_test_env_lock().lock().await;
    let root = tempfile::tempdir_in(env::var("HOME").unwrap()).unwrap();
    let file = root.path().join("owned.txt");
    fs::write(&file, "external edit").unwrap();
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let human = db
        .create_human_user("file-editor", "password-123")
        .await
        .unwrap();
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let token = session_token(&human);
    let draft = json!({"path": file, "content": "draft", "original_content": "original"});

    for (payload, auth, expected) in [
        (draft.clone(), false, StatusCode::UNAUTHORIZED),
        (
            json!({"path": file, "content": "draft"}),
            true,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"path": file, "content": "draft", "original_content": "x".repeat(1_048_577)}),
            true,
            StatusCode::BAD_REQUEST,
        ),
        (draft, true, StatusCode::CONFLICT),
    ] {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/filesystem/write")
            .header(CONTENT_TYPE, "application/json");
        if auth {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(payload.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let bytes = to_bytes(response.into_body(), 2_097_152).await.unwrap();
        if expected == StatusCode::CONFLICT {
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["current_content"], "external edit");
        } else {
            assert!(!String::from_utf8_lossy(&bytes).contains("external edit"));
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "external edit");
    }

    fs::write(&file, vec![b'x'; 1_048_577]).unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/filesystem/write")
                .header(CONTENT_TYPE, "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    json!({"path": file, "content": "draft", "original_content": "original"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(fs::metadata(file).unwrap().len(), 1_048_577);
}
