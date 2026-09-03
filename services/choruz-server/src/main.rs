//! `choruz-server` — headless host used as the remote side of an
//! SSH-tunneled connection, à la `vscode-server`.
//!
//! Lifecycle:
//!   1. Resolve migrations dir (next to the binary if deployed by the
//!      local client, else the workspace's).
//!   2. Start embedded Postgres (data dir under `~/Library/Application
//!      Support/choruz/` on macOS / `~/.local/share/choruz/` on Linux).
//!   3. Spawn choruz-api-gateway (binds :3000) + choruz-pipeline (binds :3020).
//!      Next.js is intentionally NOT started — the CLIENT renders the UI
//!      and proxies its `/api/v1/*` calls across the SSH tunnel.
//!   4. Wait for both services' versioned `/readyz` contracts, then emit
//!      `CHORUZ_LISTENING=<gateway_port>\n` so the local client can establish
//!      the tunnel.
//!   5. Block until SIGINT / SIGTERM, then stop children gracefully + stop pg.

use std::sync::Arc;

use choruz_supervisor::{pg, supervisor};

fn main() {
    if let Err(error) = choruz_infrastructure::init_tracing("choruz-server") {
        eprintln!("invalid logging configuration: {error}");
        std::process::exit(2);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

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

    let pg_handle = match rt.block_on(pg::EmbeddedPg::setup_and_start(&migrations_dir)) {
        Ok(pg) => Arc::new(pg),
        Err(e) => {
            tracing::error!(error = %e, "embedded postgres failed to start");
            std::process::exit(1);
        }
    };

    let sup = Arc::new(supervisor::Supervisor::new());
    if let Err(e) = sup.start_backend(&pg_handle.database_url) {
        tracing::error!(error = %e, "backend spawn failed");
        std::process::exit(1);
    }
    sup.start_child_monitor();

    // The actual port choruz-api-gateway listens on. For now it's fixed to 3000
    // in `Supervisor::start_backend`. If/when we make that random, we'll surface it from
    // the supervisor and print the real value here. The handshake line
    // is what the SSH client greps for.
    const GATEWAY_PORT: u16 = 3000;
    println!("CHORUZ_LISTENING={GATEWAY_PORT}");
    // Flush explicitly — SSH clients read line-by-line and we don't want
    // them blocked behind libc's line-buffering heuristics.
    use std::io::Write;
    let _ = std::io::stdout().flush();
    tracing::info!(
        port = GATEWAY_PORT,
        "choruz-server ready; blocking on signal"
    );

    // Block until the process is asked to exit. Converging cleanup paths:
    //   - SIGINT / SIGTERM handled by ctrlc
    //   - Drop on Supervisor kills children if we unwind normally
    //   - `pg.stop()` called explicitly so the pg_ctl stop doesn't race
    //     against the tokio runtime tearing down.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s = Arc::clone(&stop);
    if let Err(e) = ctrlc::set_handler(move || {
        s.store(true, std::sync::atomic::Ordering::SeqCst);
    }) {
        tracing::warn!(error = %e, "signal handler registration failed; SIGINT won't be caught cleanly");
    }

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
    rt.block_on(pg_handle.stop());
    if backend_failed {
        std::process::exit(1);
    }
}
