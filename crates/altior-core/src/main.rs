//! Altior Core composition root and daemon CLI entrypoint (P1.3 / ADR 0006).
//!
//! Parses CLI options (`--daemon`, `--data-dir`, `--endpoint`, `--discovery`).
//! If `--daemon` is omitted, prints the standard version/capability banner
//! to preserve compatibility with early contract tests.
//!
//! When `--daemon` is specified, runs a resident daemon listening on real OS
//! transport (Windows named pipes / Unix domain sockets), publishing an atomic
//! discovery file with a cryptographically secure per-launch capability token.

use std::path::Path;
use std::str::FromStr;

use altior_core::application::daemon::CoreDaemonConfig;
use altior_core::application::{CoreApplication, CoreDaemon};
use altior_core::runtime::adapters::acp::AcpHarnessAdapter;
use altior_core::runtime::adapters::storage::StoreCheckpointAdapter;
use altior_ipc::auth::{generate_instance_id, generate_launch_token};
use altior_ipc::{
    Endpoint, EndpointDiscovery, LaunchCredentials, LocalListener, cleanup_stale_discovery,
    write_discovery_file,
};
use altior_protocol::{CapabilitySet, CoreHello, ProductVersion, SUPPORTED_PROTOCOL_VERSIONS};
use altior_storage::Store;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let config =
        CoreDaemonConfig::parse_args(&args).map_err(|err| format!("Argument error: {err}"))?;

    let core_version = ProductVersion::from_str(env!("CARGO_PKG_VERSION"))
        .map_err(|err| format!("Failed to parse Core version: {err}"))?;

    if !config.is_daemon {
        print_banner(core_version);
        return Ok(());
    }

    run_daemon(&config, core_version)
}

fn print_banner(core_version: ProductVersion) {
    let hello = CoreHello {
        supported_versions: SUPPORTED_PROTOCOL_VERSIONS,
        core_version,
        capabilities: CapabilitySet::new(),
    };
    println!(
        "Altior Core {} (IPC {}), capabilities declared: {}",
        hello.core_version,
        hello.supported_versions,
        hello.capabilities.len(),
    );
}

fn create_credentials() -> Result<LaunchCredentials, String> {
    let instance_id = generate_instance_id()
        .map_err(|err| format!("Failed to generate Core instance ID from OS RNG: {err}"))?;
    let launch_token = generate_launch_token()
        .map_err(|err| format!("Failed to generate Core launch token from OS RNG: {err}"))?;

    Ok(LaunchCredentials {
        instance_id,
        launch_token,
    })
}

fn open_store(data_dir: Option<&Path>) -> Result<Store, String> {
    let dir = data_dir.unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|err| {
        format!(
            "Failed to create Core data directory {}: {err}",
            dir.display()
        )
    })?;

    let db_path = dir.join("altior-core.sqlite");
    Store::open(&db_path).map_err(|err| {
        format!(
            "Failed to open SQLite database at {}: {err}",
            db_path.display()
        )
    })
}

fn resolve_endpoint(config: &CoreDaemonConfig) -> Result<Endpoint, String> {
    if let Some(ref ep_str) = config.endpoint {
        if cfg!(windows) {
            Endpoint::windows_pipe(ep_str)
                .map_err(|err| format!("Invalid Windows pipe endpoint '{ep_str}': {err}"))
        } else {
            Endpoint::unix_socket(ep_str)
                .map_err(|err| format!("Invalid Unix socket endpoint '{ep_str}': {err}"))
        }
    } else {
        Endpoint::default_for_current_user()
            .map_err(|err| format!("Failed to derive default user endpoint: {err}"))
    }
}

fn publish_discovery(
    discovery_path: Option<&Path>,
    credentials: &LaunchCredentials,
    endpoint: &Endpoint,
) -> Result<(), String> {
    if let Some(discovery_file) = discovery_path {
        let discovery = EndpointDiscovery {
            instance_id: credentials.instance_id.clone(),
            endpoint: endpoint.clone(),
            launch_token: credentials.launch_token.clone(),
        };
        write_discovery_file(discovery_file, &discovery).map_err(|err| {
            format!(
                "Failed to write atomic discovery file {}: {err}",
                discovery_file.display()
            )
        })?;
    }
    Ok(())
}

fn run_daemon(config: &CoreDaemonConfig, _core_version: ProductVersion) -> Result<(), String> {
    // 1. Generate CSPRNG instance identifier and per-launch capability token
    let credentials = create_credentials()?;

    // 2. Open SQLite persistence store (fail closed: no in-memory fallback in production daemon)
    let store = open_store(config.data_dir.as_deref())?;
    let harness = AcpHarnessAdapter::new();
    let checkpoint = StoreCheckpointAdapter::new(store);
    let mut app = CoreApplication::new(harness, checkpoint, credentials.clone());

    if let Ok(recovery) = app.on_startup()
        && recovery.indeterminate_checkpoints_count > 0
    {
        eprintln!(
            "Core startup recovery: {} indeterminate checkpoints detected across store (auto-resend strictly prohibited)",
            recovery.indeterminate_checkpoints_count
        );
    }

    // 3. Resolve local OS IPC endpoint
    let endpoint = resolve_endpoint(config)?;

    // 4. Bind real OS LocalListener (named pipe on Windows, UDS on Unix)
    let listener = LocalListener::bind(&endpoint).map_err(|err| {
        format!(
            "Failed to bind local listener at {}: {err}",
            endpoint.address()
        )
    })?;

    // 5. Atomically publish discovery file if path provided
    publish_discovery(config.discovery_path.as_deref(), &credentials, &endpoint)?;

    println!(
        "Altior Core daemon started: instance={}, endpoint={}",
        credentials.instance_id.as_str(),
        endpoint.address()
    );

    // 6. Run Core daemon loop continuously until shutdown
    let mut daemon = CoreDaemon::new(app, listener);
    let loop_res = daemon.run_loop(std::time::Duration::from_millis(20));

    if let Some(ref discovery_file) = config.discovery_path {
        let _ = cleanup_stale_discovery(discovery_file);
    }

    loop_res.map_err(|err| format!("Core daemon loop exited with error: {err:?}"))
}
