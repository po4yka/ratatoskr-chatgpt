//! Integration tests for schema application. They skip unless
//! `CHATGPT_TEST_DATABASE_URL` is set; CI always sets it.

use secrecy::SecretString;

/// Every relation the first-version definition declares.
const DECLARED: [&str; 15] = [
    "chatgpt_archive.accounts",
    "chatgpt_archive.exports",
    "chatgpt_archive.import_runs",
    "chatgpt_archive.projects",
    "chatgpt_archive.conversations",
    "chatgpt_archive.messages",
    "chatgpt_archive.message_relations",
    "chatgpt_archive.content_parts",
    "chatgpt_archive.assets",
    "chatgpt_archive.revisions",
    "chatgpt_archive.raw_records",
    "chatgpt_archive.completeness_reports",
    "chatgpt_archive.tombstones",
    "chatgpt_archive.outbox_events",
    "chatgpt_archive.inbox_events",
];

fn test_url() -> Option<String> {
    // The runner exports the database location; reading it here is the whole
    // point of this harness, so the disallowed-method rule is lifted for this
    // one helper instead of for the workspace.
    #[allow(
        clippy::disallowed_methods,
        reason = "the test harness reads the database URL the runner exports"
    )]
    let url = std::env::var("CHATGPT_TEST_DATABASE_URL").ok();
    url.filter(|url| !url.trim().is_empty())
}

async fn connected_database(
    url: &str,
) -> Result<ratatoskr_chatgpt_archive::persistence::Database, Box<dyn std::error::Error>> {
    let storage = ratatoskr_chatgpt_archive::config::StorageConfig {
        blob_root: None,
        database_url: Some(SecretString::from(url.to_owned())),
    };
    let limits = ratatoskr_chatgpt_archive::config::Limits {
        database_connections: 2,
        database_acquire_timeout_ms: 5_000,
        shutdown_timeout_ms: 5_000,
    };
    Ok(ratatoskr_chatgpt_archive::persistence::Database::connect(&storage, &limits).await?)
}

/// Every relation the definition declares exists after one application.
#[tokio::test]
async fn applying_creates_declared_relations() -> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;

    for relation in DECLARED {
        let present: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(relation)
            .fetch_one(database.pool())
            .await?;
        assert!(present, "{relation} must exist after apply_schema");
    }
    Ok(())
}

/// A second application succeeds and changes nothing observable.
#[tokio::test]
async fn second_application_changes_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;

    let before: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'chatgpt_archive' ORDER BY table_name, ordinal_position",
    )
    .fetch_all(database.pool())
    .await?;

    database.apply_schema().await?;

    let after: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'chatgpt_archive' ORDER BY table_name, ordinal_position",
    )
    .fetch_all(database.pool())
    .await?;

    assert_eq!(before, after, "a re-application must not change the shape");
    Ok(())
}
