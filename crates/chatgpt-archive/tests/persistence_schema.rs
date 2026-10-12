//! Integration tests for schema application. They skip unless
//! `CHATGPT_TEST_DATABASE_URL` is set; CI always sets it.

mod common;

use ratatoskr_chatgpt_archive::portable_export::PortableExportFilter;
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
        receipt_staging_root: None,
    };
    let limits = ratatoskr_chatgpt_archive::config::Limits {
        database_connections: 2,
        database_acquire_timeout_ms: 5_000,
        shutdown_timeout_ms: 5_000,
        max_archive_bytes: 17_179_869_184,
        max_archive_entries: 10_000,
        max_archive_entry_bytes: 2_147_483_648,
        max_archive_decompressed_bytes: 34_359_738_368,
        max_archive_compression_ratio: 100,
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

/// The current schema carries every relation and identity guard needed by
/// privacy erasure and parser reprocessing; applying it twice stays safe.
#[tokio::test]
async fn schema_exposes_privacy_reparse_and_provenance_relations()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;
    database.apply_schema().await?;

    let relations = [
        "chatgpt_archive.export_entity_observations",
        "chatgpt_archive.extracted_artifacts",
        "chatgpt_archive.privacy_deletion_requests",
        "chatgpt_archive.privacy_deletion_items",
        "chatgpt_archive.privacy_deletion_audits",
        "chatgpt_archive.reparse_runs",
        "chatgpt_archive.parser_migration_reports",
    ];
    for relation in relations {
        let present: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(relation)
            .fetch_one(database.pool())
            .await?;
        assert!(present, "{relation} must exist in the current schema");
    }

    let constraints: Vec<String> = sqlx::query_scalar(
        "SELECT conname FROM pg_constraint
         WHERE connamespace = 'chatgpt_archive'::regnamespace
         ORDER BY conname",
    )
    .fetch_all(database.pool())
    .await?;
    for required in [
        "export_observation_identity_unique",
        "extracted_artifact_identity_unique",
        "privacy_deletion_scope_shape",
        "privacy_deletion_request_item_identity_unique",
        "privacy_deletion_audit_request_unique",
        "reparse_execution_identity_unique",
        "parser_migration_operation_identity_unique",
    ] {
        assert!(
            constraints.iter().any(|constraint| constraint == required),
            "missing lifecycle constraint {required}"
        );
    }

    let outbox_deduplicates: bool = sqlx::query_scalar(
        "SELECT to_regclass('chatgpt_archive.outbox_deduplication_key_unique') IS NOT NULL",
    )
    .fetch_one(database.pool())
    .await?;
    assert!(
        outbox_deduplicates,
        "privacy tombstones require a durable outbox deduplication key"
    );
    Ok(())
}

// --- receipt-shape constraints (authenticated-archive-receipt) ---

use uuid::Uuid;

async fn insert_account(
    database: &ratatoskr_chatgpt_archive::persistence::Database,
    external_ref: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(id)
        .bind(external_ref)
        .execute(database.pool())
        .await?;
    Ok(id)
}

#[allow(
    clippy::too_many_arguments,
    reason = "an export row carries its evidence fields explicitly"
)]
async fn insert_export(
    database: &ratatoskr_chatgpt_archive::persistence::Database,
    id: Uuid,
    account: Option<Uuid>,
    digest: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        "INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', '{}', $4, 10)",
    )
    .bind(id)
    .bind(Uuid::now_v7())
    .bind(account)
    .bind(digest)
    .execute(database.pool())
    .await?;
    Ok(())
}

fn digest(hex_prefix: char) -> String {
    // Random per call so parallel and repeated runs never collide on the
    // per-account digest uniqueness while still spelling valid SHA-256 hex.
    let mut hex = String::with_capacity(64);
    hex.push(hex_prefix);
    while hex.len() < 64 {
        hex.push_str(&Uuid::now_v7().simple().to_string());
    }
    hex.truncate(64);
    hex
}

/// Digest uniqueness is per tenant: two accounts may hold equal digests, one
/// account cannot hold the same digest twice.
#[tokio::test]
async fn equal_digests_coexist_across_accounts_only() -> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;

    // Random tenant refs and digest so repeated runs against the shared
    // test database never collide with rows a previous run left behind.
    let run_tag = Uuid::now_v7().simple().to_string();
    let account_a = insert_account(&database, &format!("receipt-test-{run_tag}-a")).await?;
    let account_b = insert_account(&database, &format!("receipt-test-{run_tag}-b")).await?;
    let shared = digest('d');

    insert_export(&database, Uuid::now_v7(), Some(account_a), &shared).await?;
    insert_export(&database, Uuid::now_v7(), Some(account_b), &shared).await?;

    let duplicate = insert_export(&database, Uuid::now_v7(), Some(account_a), &shared).await;
    assert!(
        duplicate.is_err(),
        "one account cannot hold the same digest twice"
    );
    Ok(())
}

/// The portable read model never selects normalized projections from another tenant.
#[tokio::test]
async fn portable_export_read_model_is_scoped_to_the_requested_tenant()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;
    let run_tag = Uuid::now_v7().simple().to_string();
    let alpha_ref = format!("portable-export-{run_tag}-alpha");
    let beta_ref = format!("portable-export-{run_tag}-beta");
    let alpha = insert_account(&database, &alpha_ref).await?;
    let beta = insert_account(&database, &beta_ref).await?;
    let alpha_export = Uuid::now_v7();
    let beta_export = Uuid::now_v7();
    insert_export(&database, alpha_export, Some(alpha), &digest('a')).await?;
    insert_export(&database, beta_export, Some(beta), &digest('b')).await?;
    let alpha_project = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.projects (id, account_id, external_id, title, last_seen_export) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(alpha_project)
    .bind(alpha)
    .bind("project-alpha")
    .bind("Alpha")
    .bind(alpha_export)
    .execute(database.pool())
    .await?;
    let alpha_conversation = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.conversations (id, project_id, account_id, external_id, title, last_seen_export) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(alpha_conversation)
    .bind(alpha_project)
    .bind(alpha)
    .bind("conversation-alpha")
    .bind("Alpha conversation")
    .bind(alpha_export)
    .execute(database.pool())
    .await?;
    let alpha_message = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.messages (id, conversation_id, external_id, role, provider_metadata) VALUES ($1, $2, $3, 'user', '{}')",
    )
    .bind(alpha_message)
    .bind(alpha_conversation)
    .bind("message-alpha")
    .execute(database.pool())
    .await?;
    sqlx::query(
        "INSERT INTO chatgpt_archive.content_parts (id, message_id, ordinal, part_kind, payload) VALUES ($1, $2, 0, 'text', '{\"text\": \"alpha content\"}')",
    )
    .bind(Uuid::now_v7())
    .bind(alpha_message)
    .execute(database.pool())
    .await?;
    sqlx::query(
        "INSERT INTO chatgpt_archive.projects (id, account_id, external_id, title, last_seen_export) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(beta)
    .bind("project-beta")
    .bind("Beta")
    .bind(beta_export)
    .execute(database.pool())
    .await?;

    let state = database
        .load_portable_archive_state(&PortableExportFilter {
            account_external_ref: alpha_ref,
            project_external_id: None,
            observed_from_rfc3339: None,
            observed_to_rfc3339: None,
        })
        .await?;

    assert_eq!(state.projects.len(), 1);
    assert_eq!(state.projects[0].external_id, "project-alpha");
    assert_eq!(state.conversations.len(), 1);
    assert_eq!(state.conversations[0].external_id, "conversation-alpha");
    assert_eq!(
        state.conversations[0].payload["messages"][0]["parts"][0]["payload"]["text"],
        "alpha content",
        "the read model must preserve ordered normalized message parts"
    );
    assert!(state.assets.is_empty());
    Ok(())
}

/// An export row always names its owning account.
#[tokio::test]
async fn export_row_requires_an_owner_account() -> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;

    let orphaned = insert_export(&database, Uuid::now_v7(), None, &digest('e')).await;
    assert!(
        orphaned.is_err(),
        "an export without an owner must be refused"
    );
    Ok(())
}

/// A run exists before its export row does, and only declared states are
/// admitted.
#[tokio::test]
async fn run_exists_before_export_and_rejects_unknown_states()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = connected_database(&url).await?;
    database.apply_schema().await?;

    // A run may exist before any export row materializes.
    let run_id = Uuid::now_v7();
    let early = sqlx::query(
        "INSERT INTO chatgpt_archive.import_runs (id, parser_version, state) VALUES ($1, NULL, 'received')",
    )
    .bind(run_id)
    .execute(database.pool())
    .await;
    assert!(
        early.is_ok(),
        "a run precedes its export: {:?}",
        early.err().map(|error| error.to_string())
    );

    // The state column admits exactly the declared set.
    let bogus = sqlx::query(
        "INSERT INTO chatgpt_archive.import_runs (id, parser_version, state) VALUES ($1, NULL, 'unknown_stage')",
    )
    .bind(Uuid::now_v7())
    .execute(database.pool())
    .await;
    assert!(bogus.is_err(), "undeclared states must be refused");

    // The digest and length a run captures at hashed persist.
    let recorded_digest = digest('f');
    sqlx::query(
        "UPDATE chatgpt_archive.import_runs SET state = 'hashed', sha256_hex = $2, byte_length = 42 WHERE id = $1",
    )
    .bind(run_id)
    .bind(&recorded_digest)
    .execute(database.pool())
    .await?;
    let recorded: Option<(String, i64)> = sqlx::query_as(
        "SELECT sha256_hex, byte_length FROM chatgpt_archive.import_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(database.pool())
    .await?;
    assert_eq!(recorded, Some((recorded_digest, 42)));
    Ok(())
}

/// Closes the statement collected so far, keeping it when it adds a column.
fn close_statement(found: &mut Vec<(usize, String)>, piece: &mut String, line: usize) {
    let statement = piece.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowered = statement.to_lowercase();
    if lowered.contains("alter table") && lowered.contains("add column") {
        found.push((line, statement));
    }
    piece.clear();
}

/// The line and text of every `ALTER TABLE ... ADD COLUMN` statement in `schema`.
///
/// Comments are dropped, statements are cut at `;`, and whitespace is collapsed, so a statement
/// split across lines still matches. A `DO $$ ... $$` block is cut at its inner `;` too, which is
/// harmless: only a piece that holds both `alter table` and `add column` counts, and the
/// constraint additions that close a foreign-key cycle hold neither `add column`.
fn column_additions(schema: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut piece = String::new();
    let mut piece_line = 0;
    for (index, line) in schema.lines().enumerate() {
        let code = line.split("--").next().unwrap_or_default();
        for (position, part) in code.split(';').enumerate() {
            if position > 0 {
                close_statement(&mut found, &mut piece, piece_line);
            }
            if piece.trim().is_empty() && !part.trim().is_empty() {
                piece_line = index + 1;
            }
            piece.push(' ');
            piece.push_str(part);
        }
    }
    close_statement(&mut found, &mut piece, piece_line);
    found
}

/// XR-021 CONTRACTS.md R4-10: the one current definition declares each column in its `CREATE
/// TABLE`; a column added afterwards is migration code, which the development status forbids.
///
/// Sanity check of the scan on the real file before the fix (`git show d994f8b:schema.sql`): it
/// reports exactly seven statements, at lines 75 (`import_runs.parser_name`), 312, 314, 316 and
/// 318 (`outbox_events.deduplication_key`, `tenant_id`, `export_id`, `undeliverable_at`) and 343
/// and 345 (`inbox_events.tenant_id`, `export_id`).
#[test]
fn the_schema_adds_no_column_after_creating_a_table() {
    let schema = include_str!("../../../schema.sql");
    let found = column_additions(schema);
    assert!(
        found.is_empty(),
        "schema.sql adds columns after creating their tables: {found:#?}"
    );
}

/// The scan itself recognises an addition split over lines and ignores a constraint and a comment.
#[test]
fn the_column_scan_finds_an_addition_and_skips_everything_else() {
    let sample = "CREATE TABLE a (id INT);\n\
        -- ALTER TABLE a ADD COLUMN in_a_comment INT;\n\
        ALTER TABLE a\n    ADD COLUMN IF NOT EXISTS b INT;\n\
        DO $$ BEGIN ALTER TABLE a ADD CONSTRAINT c CHECK (id > 0); END $$;\n";
    let found = column_additions(sample);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].0, 3,
        "the statement starts at its ALTER TABLE line"
    );
}

/// Applying the definition twice to one fresh database makes the server say nothing about a column
/// that already exists. `CREATE TABLE IF NOT EXISTS` still says a relation exists; only a column
/// notice betrays an `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`.
#[tokio::test]
async fn applying_the_schema_twice_emits_no_already_exists_notice() {
    let logs = common::logs::ThreadLogs::install();
    let database = common::DisposableDatabase::create().await;
    database
        .database
        .apply_schema()
        .await
        .expect("the schema applies a second time");
    let notices: Vec<String> = logs
        .take_lines()
        .into_iter()
        .filter(|line| line.contains("column") && line.contains("already exists"))
        .collect();
    assert!(
        notices.is_empty(),
        "an ALTER TABLE re-adds a declared column: {notices:#?}"
    );
    database.discard().await;
}
