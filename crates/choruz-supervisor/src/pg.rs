//! An exact PostgreSQL distribution with private data and migration ownership.
//! Installation and data live under `CHORUZ_DATA_DIR` (the OS data directory's
//! `choruz` folder by default). First launch downloads the selected distribution;
//! incompatible data is rejected before setup. Startup applies unapplied migrations,
//! and the handle owns shutdown. The returned URL reflects the configured port.

use postgresql_embedded::{PostgreSQL, Settings, SettingsBuilder, VersionReq};
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS: u64 = 60;

fn embedded_settings(
    install_dir: PathBuf,
    data_dir: PathBuf,
    password_file: PathBuf,
) -> Result<SettingsBuilder, String> {
    Ok(SettingsBuilder::new()
        .version(
            VersionReq::parse("=16.15.0")
                .map_err(|e| format!("embedded PostgreSQL version: {e}"))?,
        )
        .installation_dir(install_dir)
        .data_dir(data_dir)
        .password_file(password_file))
}

fn postgres_command_timeout(value: Option<&str>) -> Duration {
    let seconds = value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS);
    Duration::from_secs(seconds)
}

pub struct EmbeddedPg {
    inner: PostgreSQL,
    pub database_url: String,
}

impl EmbeddedPg {
    /// Start Postgres, ensure the `choruz` database + migrations exist,
    /// and return a handle that can be dropped to shut the server down.
    ///
    /// `migrations_dir` is the directory containing `*.sql` files: next to
    /// a deployed binary, or `<workspace>/migrations/` in dev. The caller
    /// resolves which is available (see `choruz-server`'s `main.rs`).
    pub async fn setup_and_start(migrations_dir: &Path) -> Result<Self, String> {
        let base = std::env::var_os("CHORUZ_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|path| path.join("choruz")))
            .ok_or_else(|| "no OS data directory available".to_string())?;
        let data_dir: PathBuf = base.join("pgdata");
        let install_dir: PathBuf = base.join("pg-install");
        let password_file: PathBuf = base.join("pgpass");

        if let Some(parent) = data_dir.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }

        tracing::info!(data_dir = %data_dir.display(), "embedded postgres booting");

        let settings = embedded_settings(install_dir, data_dir, password_file)?
            .port(
                std::env::var("CHORUZ_POSTGRES_PORT")
                    .ok()
                    .map(|value| value.parse::<u16>())
                    .transpose()
                    .map_err(|_| "CHORUZ_POSTGRES_PORT must be a valid port".to_string())?
                    .unwrap_or(5433),
            )
            .username("postgres")
            .password(
                std::env::var("CHORUZ_POSTGRES_PASSWORD")
                    .unwrap_or_else(|_| "postgres".to_string()),
            )
            .temporary(false)
            .timeout(Some(postgres_command_timeout(
                std::env::var("CHORUZ_POSTGRES_COMMAND_TIMEOUT_SECS")
                    .ok()
                    .as_deref(),
            )))
            .build();

        Self::start(settings, migrations_dir).await
    }

    async fn start(settings: Settings, migrations_dir: &Path) -> Result<Self, String> {
        validate_data_major(&settings.data_dir).await?;
        let mut pg = PostgreSQL::new(settings);
        pg.setup().await.map_err(|e| format!("pg setup: {e}"))?;
        pg.start().await.map_err(|e| format!("pg start: {e}"))?;

        let db_name = "choruz";
        let exists = pg
            .database_exists(db_name)
            .await
            .map_err(|e| format!("pg database_exists: {e}"))?;
        if !exists {
            pg.create_database(db_name)
                .await
                .map_err(|e| format!("pg create_database: {e}"))?;
            tracing::info!(db = db_name, "created database");
        } else {
            tracing::info!(db = db_name, "database already exists");
        }

        let database_url = pg.settings().url(db_name);
        tracing::info!(port = pg.settings().port, "embedded postgres ready");

        apply_migrations(&database_url, migrations_dir).await?;

        Ok(Self {
            inner: pg,
            database_url,
        })
    }

    pub async fn stop(&self) {
        // Best-effort — on rapid shutdown the server might already be gone.
        match self.inner.stop().await {
            Ok(()) => tracing::info!("embedded postgres stopped"),
            Err(e) => tracing::warn!(error = %e, "pg stop failed (probably already exited)"),
        }
    }
}

async fn validate_data_major(data_dir: &Path) -> Result<(), String> {
    let path = data_dir.join("PG_VERSION");
    match tokio::fs::read_to_string(&path).await {
        Ok(version) if version.trim() == "16" => Ok(()),
        Ok(version) => Err(format!(
            "{} contains PostgreSQL {}; Choruz requires PostgreSQL 16. The data was not changed. Use the matching PostgreSQL server to make a backup, then restore it into a separate PostgreSQL 16 database after checking compatibility. Do not delete this data directory to retry.",
            data_dir.display(),
            version.trim()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "read {} before PostgreSQL startup: {error}",
            path.display()
        )),
    }
}

/// Apply every `*.sql` in `migrations_dir` that hasn't been applied yet,
/// tracked by a `_migrations (filename TEXT PK)` table — same contract as
/// `infra/host/migrate.sh` so switching between that and this codepath
/// doesn't re-apply migrations or confuse the tracker.
async fn apply_migrations(database_url: &str, migrations_dir: &Path) -> Result<(), String> {
    tracing::info!("apply_migrations: connect start");
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls)
        .await
        .map_err(|e| format!("connect for migrations: {e}"))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::warn!(error = %e, "migration connection dropped");
        }
    });
    tracing::info!("apply_migrations: connected");

    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS _migrations \
             (filename TEXT PRIMARY KEY, applied_at TIMESTAMPTZ DEFAULT NOW())",
        )
        .await
        .map_err(|e| format!("create _migrations: {e}"))?;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS _migration_statement_progress \
             (filename TEXT NOT NULL, statement_index INTEGER NOT NULL, \
              PRIMARY KEY (filename, statement_index))",
        )
        .await
        .map_err(|e| format!("create _migration_statement_progress: {e}"))?;
    tracing::info!("apply_migrations: _migrations table ensured");

    // Sorted alphabetically — matches the shell script's `for f in
    // migrations/*.sql` glob order (V001 < V002 < ... < V0NN; the 00NN
    // files come first lexically, which is what we want since they were
    // the earlier numbering scheme).
    let mut entries: Vec<PathBuf> = std::fs::read_dir(migrations_dir)
        .map_err(|e| format!("read {}: {e}", migrations_dir.display()))?
        .filter_map(|r| r.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sql"))
        .collect();
    entries.sort();
    tracing::info!(
        total = entries.len(),
        "apply_migrations: enumerated sql files"
    );

    let mut applied_count = 0usize;
    for path in entries {
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("bad filename {}", path.display()))?
            .to_string();

        let row = client
            .query_one(
                "SELECT count(*)::BIGINT FROM _migrations WHERE filename = $1",
                &[&filename],
            )
            .await
            .map_err(|e| format!("check {filename}: {e}"))?;
        let count: i64 = row.get(0);
        if count > 0 {
            continue;
        }

        let sql = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("read {filename}: {e}"))?;

        execute_migration(&client, &filename, &sql)
            .await
            .map_err(|e| format!("apply {filename}: {e}"))?;

        client
            .execute(
                "INSERT INTO _migrations (filename) VALUES ($1)",
                &[&filename],
            )
            .await
            .map_err(|e| format!("track {filename}: {e}"))?;

        applied_count += 1;
        tracing::info!(migration = %filename, "applied");
    }

    tracing::info!(applied_count, "migrations done");
    Ok(())
}

/// PostgreSQL forbids `CREATE/DROP INDEX CONCURRENTLY` inside a transaction.
/// `batch_execute` may execute a multi-statement migration in an implicit
/// transaction, so run concurrent-index statements one by one. This mirrors
/// the repository's PostgreSQL integration-test migrator.
async fn execute_migration(
    client: &tokio_postgres::Client,
    filename: &str,
    sql: &str,
) -> Result<(), tokio_postgres::Error> {
    if !contains_concurrently(sql) {
        return client.batch_execute(sql).await;
    }

    // A concurrent migration cannot be atomic. Record each successfully
    // executed statement so a later launch resumes after a failed statement
    // instead of replaying earlier DDL (which is often non-idempotent).
    for (statement_index, statement) in sql_statements(sql).into_iter().enumerate() {
        let applied = client
            .query_opt(
                "SELECT 1 FROM _migration_statement_progress \
                 WHERE filename = $1 AND statement_index = $2",
                &[&filename, &(statement_index as i32)],
            )
            .await?
            .is_some();
        if applied {
            continue;
        }
        client.batch_execute(&statement).await?;
        client
            .execute(
                "INSERT INTO _migration_statement_progress (filename, statement_index) \
                 VALUES ($1, $2)",
                &[&filename, &(statement_index as i32)],
            )
            .await?;
    }
    Ok(())
}

fn contains_concurrently(sql: &str) -> bool {
    scan_sql(sql)
        .code
        .split(|character: char| !character.is_ascii_alphabetic())
        .any(|word| word.eq_ignore_ascii_case("concurrently"))
}

fn sql_statements(sql: &str) -> Vec<String> {
    scan_sql(sql).statements
}

struct SqlScan {
    statements: Vec<String>,
    code: String,
}

/// Split migration SQL on real statement terminators while producing a copy
/// whose literals and comments are blanked. PostgreSQL permits semicolons and
/// keywords in both places, so neither `split(';')` nor substring matching is
/// safe here.
fn scan_sql(sql: &str) -> SqlScan {
    let mut statements = Vec::new();
    let mut code = String::with_capacity(sql.len());
    let mut statement_start = 0;
    let mut code_start = 0;
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut state = SqlState::Normal;

    while index < bytes.len() {
        match &state {
            SqlState::Normal => match bytes[index] {
                b'\'' => {
                    state = SqlState::SingleQuote;
                    code.push(' ');
                    index += 1;
                }
                b'\"' => {
                    state = SqlState::DoubleQuote;
                    code.push(' ');
                    index += 1;
                }
                b'-' if bytes.get(index + 1) == Some(&b'-') => {
                    state = SqlState::LineComment;
                    code.push_str("  ");
                    index += 2;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    state = SqlState::BlockComment(1);
                    code.push_str("  ");
                    index += 2;
                }
                b'$' => {
                    if let Some(delimiter) = dollar_quote_delimiter(&sql[index..]) {
                        code.extend(std::iter::repeat_n(' ', delimiter.len()));
                        index += delimiter.len();
                        state = SqlState::DollarQuote(delimiter);
                    } else {
                        code.push('$');
                        index += 1;
                    }
                }
                b';' => {
                    push_statement(
                        &mut statements,
                        &sql[statement_start..index],
                        &code[code_start..],
                    );
                    code.push(' ');
                    statement_start = index + 1;
                    code_start = code.len();
                    index += 1;
                }
                character => {
                    code.push(if character.is_ascii() {
                        character as char
                    } else {
                        ' '
                    });
                    index += 1;
                }
            },
            SqlState::SingleQuote => {
                if bytes[index] == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        code.push_str("  ");
                        index += 2;
                    } else {
                        state = SqlState::Normal;
                        code.push(' ');
                        index += 1;
                    }
                } else {
                    code.push(' ');
                    index += 1;
                }
            }
            SqlState::DoubleQuote => {
                if bytes[index] == b'\"' {
                    if bytes.get(index + 1) == Some(&b'\"') {
                        code.push_str("  ");
                        index += 2;
                    } else {
                        state = SqlState::Normal;
                        code.push(' ');
                        index += 1;
                    }
                } else {
                    code.push(' ');
                    index += 1;
                }
            }
            SqlState::LineComment => {
                if bytes[index] == b'\n' {
                    state = SqlState::Normal;
                }
                code.push(' ');
                index += 1;
            }
            SqlState::BlockComment(depth) => {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    state = SqlState::BlockComment(depth + 1);
                    code.push_str("  ");
                    index += 2;
                } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = if *depth == 1 {
                        SqlState::Normal
                    } else {
                        SqlState::BlockComment(depth - 1)
                    };
                    code.push_str("  ");
                    index += 2;
                } else {
                    code.push(' ');
                    index += 1;
                }
            }
            SqlState::DollarQuote(delimiter) => {
                if bytes[index..].starts_with(delimiter.as_bytes()) {
                    code.extend(std::iter::repeat_n(' ', delimiter.len()));
                    index += delimiter.len();
                    state = SqlState::Normal;
                } else {
                    code.push(' ');
                    index += 1;
                }
            }
        }
    }

    push_statement(
        &mut statements,
        &sql[statement_start..],
        &code[code_start..],
    );
    SqlScan { statements, code }
}

fn push_statement(statements: &mut Vec<String>, statement: &str, code: &str) {
    if !code.trim().is_empty() {
        statements.push(statement.trim().to_owned());
    }
}

#[derive(Clone)]
enum SqlState {
    Normal,
    SingleQuote,
    DoubleQuote,
    LineComment,
    BlockComment(usize),
    DollarQuote(String),
}

fn dollar_quote_delimiter(input: &str) -> Option<String> {
    let end = input[1..].find('$')? + 1;
    let tag = &input[1..end];
    if tag
        .chars()
        .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        Some(input[..=end].to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS, contains_concurrently, postgres_command_timeout,
        sql_statements,
    };

    #[test]
    fn embedded_distribution_is_exact_not_an_upstream_latest_selection() {
        let settings = super::embedded_settings("install".into(), "data".into(), "pass".into())
            .unwrap()
            .build();
        assert_eq!(settings.version.to_string(), "=16.15.0");
        assert!(
            !settings
                .version
                .matches(&postgresql_embedded::Version::new(18, 6, 0))
        );
        assert!(
            !settings
                .version
                .matches(&postgresql_embedded::Version::new(16, 16, 0))
        );
    }

    #[tokio::test]
    async fn incompatible_data_is_rejected_without_modifying_it() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let version = data.join("PG_VERSION");
        tokio::fs::write(&version, "18\n").await.unwrap();
        let install = dir.path().join("install");
        let settings = super::embedded_settings(install.clone(), data, dir.path().join("pass"))
            .unwrap()
            .port(0)
            .build();
        let error = match super::EmbeddedPg::start(settings, dir.path()).await {
            Ok(pg) => {
                pg.stop().await;
                panic!("incompatible database was started");
            }
            Err(error) => error,
        };
        assert!(error.contains("PostgreSQL 18"), "{error}");
        assert!(error.contains("backup"), "{error}");
        assert_eq!(tokio::fs::read_to_string(&version).await.unwrap(), "18\n");
        assert!(
            !install.exists(),
            "incompatible data must be rejected before installation"
        );
    }

    #[tokio::test]
    async fn compatible_and_uninitialized_data_are_accepted_but_unreadable_metadata_is_not() {
        let dir = tempfile::tempdir().unwrap();
        super::validate_data_major(dir.path()).await.unwrap();
        let version = dir.path().join("PG_VERSION");
        tokio::fs::write(&version, "16\n").await.unwrap();
        super::validate_data_major(dir.path()).await.unwrap();
        std::fs::remove_file(&version).unwrap();
        std::fs::create_dir(&version).unwrap();
        assert!(
            super::validate_data_major(dir.path())
                .await
                .unwrap_err()
                .contains("read")
        );
    }

    #[tokio::test]
    async fn embedded_first_install_and_restart_preserve_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let migrations = dir.path().join("migrations");
        std::fs::create_dir(&migrations).unwrap();
        std::fs::write(
            migrations.join("001_probe.sql"),
            "CREATE TABLE install_probe (value TEXT);",
        )
        .unwrap();
        for launch in 0..2 {
            let settings = super::embedded_settings(
                dir.path().join("install"),
                dir.path().join("data"),
                dir.path().join("pgpass"),
            )
            .unwrap()
            .username("postgres")
            .password("isolated-install-probe")
            .port(0)
            .temporary(false)
            .timeout(Some(std::time::Duration::from_secs(60)))
            .build();
            let pg = super::EmbeddedPg::start(settings, &migrations)
                .await
                .unwrap();
            let database_url = pg.database_url.clone();
            let verification = tokio::spawn(async move {
                let (client, connection) =
                    tokio_postgres::connect(&database_url, tokio_postgres::NoTls)
                        .await
                        .unwrap();
                let connection = tokio::spawn(connection);
                let version: String = client
                    .query_one("SHOW server_version", &[])
                    .await
                    .unwrap()
                    .get(0);
                assert!(version.starts_with("16.15"), "{version}");
                if launch == 0 {
                    client
                        .execute("INSERT INTO install_probe VALUES ('retained')", &[])
                        .await
                        .unwrap();
                } else {
                    let retained: String = client
                        .query_one("SELECT value FROM install_probe", &[])
                        .await
                        .unwrap()
                        .get(0);
                    assert_eq!(retained, "retained");
                }
                drop(client);
                connection.await.unwrap().unwrap();
            });
            // Stop the owned server even when an assertion fails.
            let result = verification.await;
            pg.stop().await;
            result.unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("data/PG_VERSION"))
                .unwrap()
                .trim(),
            "16"
        );
    }

    #[test]
    fn postgres_commands_allow_slow_first_start_and_an_explicit_override() {
        assert_eq!(
            postgres_command_timeout(None).as_secs(),
            DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS
        );
        assert_eq!(postgres_command_timeout(Some("120")).as_secs(), 120);
        assert_eq!(
            postgres_command_timeout(Some("0")).as_secs(),
            DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS
        );
        assert_eq!(
            postgres_command_timeout(Some("invalid")).as_secs(),
            DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS
        );
    }

    #[test]
    fn separates_concurrent_index_statements_without_breaking_sql_literals() {
        assert_eq!(
            sql_statements(
                "-- comment\nCREATE INDEX CONCURRENTLY one;\nINSERT INTO notes VALUES ('one; two');\nDROP INDEX CONCURRENTLY two;"
            ),
            vec![
                "-- comment\nCREATE INDEX CONCURRENTLY one",
                "INSERT INTO notes VALUES ('one; two')",
                "DROP INDEX CONCURRENTLY two",
            ],
        );
    }

    #[test]
    fn ignores_concurrently_inside_comments_and_quoted_sql() {
        assert!(!contains_concurrently(
            "-- CONCURRENTLY\nSELECT 'CONCURRENTLY';"
        ));
        assert!(!contains_concurrently(
            "DO $fn$ BEGIN RAISE NOTICE 'CONCURRENTLY'; END $fn$;"
        ));
        assert!(contains_concurrently(
            "CREATE INDEX CONCURRENTLY idx ON widgets (id);"
        ));
    }

    #[test]
    fn keeps_semicolons_inside_comments_and_dollar_quotes_in_one_statement() {
        assert_eq!(
            sql_statements(
                "/* setup; note */ DO $body$ BEGIN PERFORM 'semi;colon'; END $body$; CREATE INDEX CONCURRENTLY idx ON widgets (id);"
            ),
            vec![
                "/* setup; note */ DO $body$ BEGIN PERFORM 'semi;colon'; END $body$",
                "CREATE INDEX CONCURRENTLY idx ON widgets (id)",
            ],
        );
    }

    #[test]
    fn accepts_non_ascii_sql_without_invalid_utf8_slices() {
        assert!(contains_concurrently(
            "CREATE INDEX CONCURRENTLY idx_é ON widgets (id);"
        ));
    }
}
