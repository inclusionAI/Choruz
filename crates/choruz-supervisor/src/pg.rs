//! Embedded Postgres — runs a real PostgreSQL 16 server out of the user's
//! data dir, so `choruz-server` starts on a host without an installed
//! PostgreSQL.
//!
//! - First launch: downloads the PG binary (~60 MB) from
//!   https://github.com/theseus-rs/postgresql-binaries into `~/.theseus/`
//!   (cached across reinstalls), runs `initdb` into our app's data dir,
//!   creates the `choruz` database, applies every migration in
//!   `migrations/*.sql`.
//! - Subsequent launches: just `pg_ctl start`, then run any *new*
//!   migrations (same tracking table shape as `infra/host/migrate.sh`).
//! - On drop / host shutdown: `pg_ctl stop` via the inner handle.
//!
//! Port: bound to 5433 (not 5432) so it can coexist with a system
//! Postgres if the user happens to have one. The actual URL given to
//! gateway/pipeline comes from `settings().url("choruz")` so if we ever
//! switch port selection strategy, downstream code doesn't care.

use postgresql_embedded::{PostgreSQL, SettingsBuilder};
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_POSTGRES_COMMAND_TIMEOUT_SECS: u64 = 60;

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
        let base = dirs::data_dir()
            .ok_or_else(|| "no OS data directory available".to_string())?
            .join("choruz");
        let data_dir: PathBuf = base.join("pgdata");
        let install_dir: PathBuf = base.join("pg-install");
        let password_file: PathBuf = base.join("pgpass");

        if let Some(parent) = data_dir.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }

        tracing::info!(data_dir = %data_dir.display(), "embedded postgres booting");

        let settings = SettingsBuilder::new()
            .installation_dir(install_dir)
            .data_dir(data_dir)
            .password_file(password_file)
            .port(5433)
            .username("postgres")
            .password("postgres")
            .temporary(false)
            .timeout(Some(postgres_command_timeout(
                std::env::var("CHORUZ_POSTGRES_COMMAND_TIMEOUT_SECS")
                    .ok()
                    .as_deref(),
            )))
            .build();

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
