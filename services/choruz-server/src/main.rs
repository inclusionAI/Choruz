//! Headless composition of the shared API, optional pipeline and database.
//! Readiness is advertised only after selected children are ready. Shutdown
//! stops owned children and embedded PostgreSQL, never an external database.

use std::sync::Arc;

use choruz_supervisor::{pg, supervisor};

fn port(name: &str, default: u16) -> u16 {
    match std::env::var(name) {
        Ok(value) => match value.parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                eprintln!("{name} must be a port between 1 and 65535");
                std::process::exit(2);
            }
        },
        Err(std::env::VarError::NotPresent) => default,
        Err(_) => {
            eprintln!("{name} must be valid text");
            std::process::exit(2);
        }
    }
}

fn main() {
    let services = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => supervisor::Services::Full,
        [flag] if flag == "--api-only" => supervisor::Services::Api,
        [flag] if flag == "--help" => {
            println!(
                "choruz-server [--api-only]\nStart the API; include the group-chat pipeline unless --api-only is supplied. CHORUZ_DATABASE_URL selects a migrated external database; otherwise start embedded PostgreSQL."
            );
            return;
        }
        _ => {
            eprintln!("usage: choruz-server [--api-only]");
            std::process::exit(2);
        }
    };
    let gateway_port = port("CHORUZ_API_PORT", 3000);
    let pipeline_port = port("CHORUZ_PIPELINE_METRICS_PORT", 3020);
    if let Err(error) = choruz_infrastructure::init_tracing("choruz-server") {
        eprintln!("invalid logging configuration: {error}");
        std::process::exit(2);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s = Arc::clone(&stop);
    if let Err(e) = ctrlc::set_handler(move || {
        s.store(true, std::sync::atomic::Ordering::SeqCst);
    }) {
        tracing::error!(error = %e, "signal handler registration failed");
        std::process::exit(1);
    }

    // Migrations: prefer the bundle path (binary sits next to them after
    // `choruz deploy`), fall back to workspace for dev runs.
    let migrations_dir = if let Ok(exe) = std::env::current_exe() {
        exe.parent()
            .map(|d| d.join("migrations"))
            .filter(|p| p.exists())
    } else {
        None
    }
    .or_else(|| {
        supervisor::Supervisor::workspace_root_static()
            .map(|ws| ws.join("migrations"))
            .filter(|p| p.exists())
    })
    .unwrap_or_else(|| {
        tracing::error!("no migrations dir found (neither beside binary nor in workspace)");
        std::process::exit(1);
    });
    tracing::info!(migrations_dir = %migrations_dir.display(), "migrations dir");

    let (database_url, pg_handle) = match std::env::var("CHORUZ_DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => (url, None),
        Err(std::env::VarError::NotPresent) => {
            match rt.block_on(pg::EmbeddedPg::setup_and_start(&migrations_dir)) {
                Ok(pg) => (pg.database_url.clone(), Some(pg)),
                Err(e) => {
                    tracing::error!(error = %e, "embedded postgres failed to start");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("CHORUZ_DATABASE_URL must be nonempty valid text");
            std::process::exit(2);
        }
    };

    if stop.load(std::sync::atomic::Ordering::SeqCst) {
        if let Some(pg) = &pg_handle {
            rt.block_on(pg.stop());
        }
        return;
    }
    let sup = Arc::new(supervisor::Supervisor::new());
    if let Err(e) = sup.start_backend(&database_url, services, gateway_port, pipeline_port) {
        tracing::error!(error = %e, "backend spawn failed");
        sup.shutdown();
        if let Some(pg) = &pg_handle {
            rt.block_on(pg.stop());
        }
        std::process::exit(1);
    }
    sup.start_child_monitor();

    // Block until the process is asked to exit. Converging cleanup paths:
    //   - SIGINT / SIGTERM handled by ctrlc
    //   - Drop on Supervisor kills children if we unwind normally
    //   - `pg.stop()` called explicitly so the pg_ctl stop doesn't race
    //     against the tokio runtime tearing down.
    // Install shutdown handling before advertising readiness to the parent.
    println!("CHORUZ_LISTENING={gateway_port}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    tracing::info!(
        port = gateway_port,
        "choruz-server ready; blocking on signal"
    );

    while !stop.load(std::sync::atomic::Ordering::SeqCst) && !sup.backend_failed() {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    let backend_failed = sup.backend_failed();
    if backend_failed {
        tracing::error!("backend child failed; terminating choruz-server");
    } else {
        tracing::info!("shutting down");
    }
    sup.shutdown();
    if let Some(pg) = &pg_handle {
        rt.block_on(pg.stop());
    }
    if backend_failed {
        std::process::exit(1);
    }
}
