use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::fs;
use std::process::{Command, Stdio};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{ActivationPolicy, Manager, PhysicalPosition, Rect};
use std::sync::Mutex;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
struct CommandResult<T>
where
    T: Serialize,
{
    ok: bool,
    message: String,
    data: Option<T>,
}

#[derive(Debug, Serialize)]
struct RedisKeysPayload {
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypesenseConfig {
    host: String,
    port: u16,
    api_key: String,
    protocol: String,
}

#[derive(Debug, Serialize)]
struct CollectionsPayload {
    collections: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredService {
    kind: String,
    name: String,
    host: String,
    port: u16,
    version: String,
    api_key: Option<String>,
    socket: Option<String>,
}

#[derive(Debug, Serialize)]
struct PortPayload {
    port: u16,
}

#[derive(Debug, Serialize)]
struct ProcessInfo {
    pid: u32,
    name: String,
    detail: String,
    port: Option<u16>,
}

#[derive(Default)]
struct AppState {
    tray: Mutex<Option<TrayIcon>>,
}

#[tauri::command]
fn test_redis_connection(host: String, port: u16) -> CommandResult<()> {
    test_tcp_connection(host, port, "Redis".to_string())
}

#[tauri::command]
fn test_tcp_connection(host: String, port: u16, label: String) -> CommandResult<()> {
    match tcp_reachable(&host, port, Duration::from_secs(2)) {
        Ok(()) => CommandResult {
            ok: true,
            message: format!("{label} is reachable at {host}:{port}."),
            data: None,
        },
        Err(error) => CommandResult {
            ok: false,
            message: format!("{label} connection failed: {error}"),
            data: None,
        },
    }
}

fn tcp_reachable(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let address = format!("{host}:{port}");
    let Ok(mut addresses) = address.to_socket_addrs() else {
        return Err(format!("Could not resolve {address}."));
    };

    let Some(socket_address) = addresses.next() else {
        return Err(format!("No socket address found for {address}."));
    };

    TcpStream::connect_timeout(&socket_address, timeout)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn test_mysql_connection(host: String, port: u16, socket: Option<String>) -> CommandResult<()> {
    if let Some(socket) = socket.filter(|value| !value.trim().is_empty()) {
        if mysqladmin_socket_ping(&socket) {
            return CommandResult {
                ok: true,
                message: format!("MySQL is alive on socket {socket}."),
                data: None,
            };
        }

        if std::path::Path::new(&socket).exists() {
            return CommandResult {
                ok: true,
                message: format!("MySQL socket is available at {socket}."),
                data: None,
            };
        }
    }

    test_tcp_connection(host, port, "MySQL".to_string())
}

fn mysqladmin_socket_ping(socket: &str) -> bool {
    Command::new(resolve_binary("mysqladmin"))
        .args([&format!("--socket={socket}"), "ping"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[tauri::command]
fn infer_mysql_port_from_socket(socket: String) -> CommandResult<PortPayload> {
    if let Some(port) = infer_mysql_port_from_config(&socket) {
        return CommandResult {
            ok: true,
            message: format!("Detected MySQL port {port} from config."),
            data: Some(PortPayload { port }),
        };
    }

    if let Some(port) = infer_mysql_port_from_mysqladmin(&socket) {
        return CommandResult {
            ok: true,
            message: format!("Detected MySQL port {port} from server variables."),
            data: Some(PortPayload { port }),
        };
    }

    CommandResult {
        ok: false,
        message: "Could not infer a MySQL port from this socket.".to_string(),
        data: None,
    }
}

fn infer_mysql_port_from_config(socket: &str) -> Option<u16> {
    let mut candidates = Vec::new();

    if let Some(prefix) = socket.strip_suffix("/var/mysql/mysql.sock") {
        candidates.push(std::path::PathBuf::from(prefix).join("etc/my.cnf"));
    }

    candidates.extend([
        std::path::PathBuf::from("/Applications/XAMPP/xamppfiles/etc/my.cnf"),
        std::path::PathBuf::from("/opt/homebrew/etc/my.cnf"),
        std::path::PathBuf::from("/usr/local/etc/my.cnf"),
        std::path::PathBuf::from("/etc/my.cnf"),
    ]);

    candidates
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find_map(|content| parse_mysql_port(&content))
}

fn infer_mysql_port_from_mysqladmin(socket: &str) -> Option<u16> {
    let output = Command::new("mysqladmin")
        .args(["--socket", socket, "variables"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_mysql_port(&String::from_utf8_lossy(&output.stdout))
}

fn parse_mysql_port(content: &str) -> Option<u16> {
    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("port") {
            let value = value.trim_start_matches([' ', '\t', '=']).trim();
            if let Ok(port) = value.parse::<u16>() {
                return Some(port);
            }
        }

        if trimmed.contains("| port") {
            let columns = trimmed
                .split('|')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();

            if columns.first() == Some(&"port") {
                if let Some(value) = columns.get(1).and_then(|value| value.parse::<u16>().ok()) {
                    return Some(value);
                }
            }
        }
    }

    None
}

#[tauri::command]
fn list_redis_keys(
    host: String,
    port: u16,
    database: u8,
    pattern: String,
) -> CommandResult<RedisKeysPayload> {
    let output = Command::new(resolve_binary("redis-cli"))
        .args([
            "-h",
            host.as_str(),
            "-p",
            &port.to_string(),
            "-n",
            &database.to_string(),
            "--scan",
            "--pattern",
            pattern.as_str(),
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let keys = String::from_utf8_lossy(&output.stdout)
                .lines()
                .take(200)
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();

            CommandResult {
                ok: true,
                message: format!("Loaded {} Redis key(s).", keys.len()),
                data: Some(RedisKeysPayload { keys }),
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CommandResult {
                ok: false,
                message: format!("redis-cli failed: {}", stderr.trim()),
                data: None,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CommandResult {
            ok: false,
            message: "redis-cli was not found in PATH.".to_string(),
            data: None,
        },
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not run redis-cli: {error}"),
            data: None,
        },
    }
}

#[tauri::command]
fn start_redis_instance(app: tauri::AppHandle, port: u16, name: String) -> CommandResult<()> {
    if tcp_reachable("127.0.0.1", port, Duration::from_millis(250)).is_ok() {
        return CommandResult {
            ok: true,
            message: format!("{name} is already running on port {port}."),
            data: None,
        };
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("service-desk"))
        .join(format!("redis-{port}"));

    if let Err(error) = fs::create_dir_all(&data_dir) {
        return CommandResult {
            ok: false,
            message: format!("Could not create Redis data directory: {error}"),
            data: None,
        };
    }

    let output = Command::new(resolve_binary("redis-server"))
        .args([
            "--port",
            &port.to_string(),
            "--daemonize",
            "yes",
            "--dir",
            &data_dir.to_string_lossy(),
            "--dbfilename",
            "dump.rdb",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => wait_for_redis_start(port, &name),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CommandResult {
                ok: false,
                message: format!("redis-server failed: {}", stderr.trim()),
                data: None,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CommandResult {
            ok: false,
            message: "redis-server was not found in PATH.".to_string(),
            data: None,
        },
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not run redis-server: {error}"),
            data: None,
        },
    }
}

fn wait_for_redis_start(port: u16, name: &str) -> CommandResult<()> {
    for _ in 0..30 {
        if tcp_reachable("127.0.0.1", port, Duration::from_millis(100)).is_ok() {
            return CommandResult {
                ok: true,
                message: format!("{name} started on port {port}."),
                data: None,
            };
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    CommandResult {
        ok: false,
        message: format!("Redis command returned success, but port {port} did not open."),
        data: None,
    }
}

fn managed_service_dir(app: &tauri::AppHandle, kind: &str, port: u16) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("service-desk"))
        .join(format!("{kind}-{port}"))
}

fn mysql_basedir(mysqld: &str) -> Option<PathBuf> {
    let path = Path::new(mysqld);
    let parent = path.parent()?;

    if parent.file_name().and_then(|name| name.to_str()) == Some("sbin") {
        return parent.parent().map(Path::to_path_buf);
    }

    parent.parent().map(Path::to_path_buf)
}

fn mysql_install_db_for_mysqld(mysqld: &str) -> Option<String> {
    let basedir = mysql_basedir(mysqld)?;
    [
        basedir.join("bin/mariadb-install-db"),
        basedir.join("bin/mysql_install_db"),
    ]
    .into_iter()
    .find(|path| path.exists())
    .map(|path| path.to_string_lossy().into_owned())
}

fn mysqld_supports_initialize(mysqld: &str) -> bool {
    Command::new(mysqld)
        .args(["--no-defaults", "--verbose", "--help"])
        .output()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            stdout.contains("--initialize") || stderr.contains("--initialize")
        })
        .unwrap_or(false)
}

#[tauri::command]
fn create_managed_service(
    app: tauri::AppHandle,
    kind: String,
    port: u16,
    api_key: Option<String>,
) -> CommandResult<()> {
    let data_dir = managed_service_dir(&app, &kind, port);

    if kind == "mysql" {
        let mut mysqld = resolve_binary("mysqld");
        if !std::path::Path::new(&mysqld).exists() {
            if let Err(message) = ensure_mysql_installed() {
                return CommandResult {
                    ok: false,
                    message,
                    data: None,
                };
            }
            mysqld = resolve_binary("mysqld");
            if !std::path::Path::new(&mysqld).exists() {
                return CommandResult {
                    ok: false,
                    message: "brew install mysql completed, but mysqld is still not in PATH."
                        .to_string(),
                    data: None,
                };
            }
        }

        if data_dir.exists() {
            if let Err(error) = fs::remove_dir_all(&data_dir) {
                return CommandResult {
                    ok: false,
                    message: format!("Could not reset managed mysql directory: {error}"),
                    data: None,
                };
            }
        }

        if let Err(error) = fs::create_dir_all(&data_dir) {
            return CommandResult {
                ok: false,
                message: format!("Could not create managed mysql directory: {error}"),
                data: None,
            };
        }

        let output = if mysqld_supports_initialize(&mysqld) {
            Command::new(&mysqld)
                .args([
                    "--no-defaults".to_string(),
                    "--initialize-insecure".to_string(),
                    format!("--datadir={}", data_dir.display()),
                ])
                .output()
        } else if let Some(install_db) = mysql_install_db_for_mysqld(&mysqld) {
            let basedir = mysql_basedir(&mysqld);
            let mut command = Command::new(&install_db);
            command.args([
                "--no-defaults".to_string(),
                format!("--datadir={}", data_dir.display()),
                "--auth-root-authentication-method=normal".to_string(),
                "--skip-test-db".to_string(),
            ]);
            if let Some(basedir) = basedir {
                command.arg(format!("--basedir={}", basedir.display()));
            }
            command.output()
        } else {
            return CommandResult {
                ok: false,
                message: format!("{mysqld} does not support --initialize-insecure and no mysql_install_db script was found."),
                data: None,
            };
        };

        match output {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let detail = format!("{}\n{}", stderr.trim(), stdout.trim());
                return CommandResult {
                    ok: false,
                    message: format!("mysql initialization failed: {}", detail.trim()),
                    data: None,
                };
            }
            Err(error) => {
                return CommandResult {
                    ok: false,
                    message: format!("Could not run mysqld: {error}"),
                    data: None,
                };
            }
        }

        let metadata = format!("kind=mysql\nport={port}\n");
        if let Err(error) = fs::write(data_dir.join("service-desk.instance"), metadata) {
            return CommandResult {
                ok: false,
                message: format!("Could not write managed mysql metadata: {error}"),
                data: None,
            };
        }

        return CommandResult {
            ok: true,
            message: format!("Managed mysql instance prepared on port {port}."),
            data: None,
        };
    }

    if let Err(error) = fs::create_dir_all(&data_dir) {
        return CommandResult {
            ok: false,
            message: format!("Could not create managed {kind} directory: {error}"),
            data: None,
        };
    }

    let metadata = match kind.as_str() {
        "redis" => format!("kind=redis\nport={port}\n"),
        "typesense" => format!(
            "kind=typesense\nport={port}\napi_key={}\n",
            api_key.unwrap_or_else(|| "change-me".to_string())
        ),
        _ => {
            return CommandResult {
                ok: false,
                message: format!("Unsupported managed service kind: {kind}"),
                data: None,
            };
        }
    };

    if let Err(error) = fs::write(data_dir.join("service-desk.instance"), metadata) {
        return CommandResult {
            ok: false,
            message: format!("Could not write managed {kind} metadata: {error}"),
            data: None,
        };
    }

    CommandResult {
        ok: true,
        message: format!("Managed {kind} instance prepared on port {port}."),
        data: None,
    }
}

#[tauri::command]
fn stop_redis_instance(host: String, port: u16) -> CommandResult<()> {
    let output = Command::new(resolve_binary("redis-cli"))
        .args(["-h", host.as_str(), "-p", &port.to_string(), "shutdown", "nosave"])
        .output();

    match output {
        Ok(output) if output.status.success() => CommandResult {
            ok: true,
            message: format!("Redis on port {port} stopped."),
            data: None,
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CommandResult {
                ok: false,
                message: format!("redis-cli failed: {}", stderr.trim()),
                data: None,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CommandResult {
            ok: false,
            message: "redis-cli was not found in PATH.".to_string(),
            data: None,
        },
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not run redis-cli: {error}"),
            data: None,
        },
    }
}

#[tauri::command]
fn delete_managed_service(app: tauri::AppHandle, kind: String, port: u16) -> CommandResult<()> {
    let data_dir = managed_service_dir(&app, &kind, port);

    match kind.as_str() {
        "redis" => {
            let _ = stop_redis_instance("127.0.0.1".to_string(), port);
        }
        "typesense" => {
            let _ = stop_typesense_instance(app.clone(), port);
        }
        "mysql" => {
            let _ = stop_mysql_instance(app.clone(), port, "managed mysql".to_string());
        }
        _ => {
            return CommandResult {
                ok: false,
                message: format!("Unsupported managed service kind: {kind}"),
                data: None,
            };
        }
    }

    if data_dir.exists() {
        if let Err(error) = fs::remove_dir_all(&data_dir) {
            return CommandResult {
                ok: false,
                message: format!("Could not delete managed {kind} directory: {error}"),
                data: None,
            };
        }
    }

    CommandResult {
        ok: true,
        message: format!("Managed {kind} instance on port {port} deleted."),
        data: None,
    }
}

#[tauri::command]
fn test_typesense_connection(
    host: String,
    port: u16,
    api_key: String,
    protocol: String,
) -> CommandResult<()> {
    check_typesense_health(TypesenseConfig {
        host,
        port,
        api_key,
        protocol,
    })
}

#[tauri::command]
fn start_typesense_instance(
    app: tauri::AppHandle,
    port: u16,
    name: String,
    api_key: String,
) -> CommandResult<()> {
    let data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("service-desk"))
        .join(format!("typesense-{port}"));

    if let Err(error) = fs::create_dir_all(&data_dir) {
        return CommandResult {
            ok: false,
            message: format!("Could not create Typesense data directory: {error}"),
            data: None,
        };
    }

    let child = Command::new("typesense-server")
        .args([
            "--data-dir",
            &data_dir.to_string_lossy(),
            "--api-key",
            api_key.as_str(),
            "--listen-port",
            &port.to_string(),
            "--enable-cors",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match child {
        Ok(child) => {
            let pid_file = data_dir.join("typesense.pid");
            let _ = fs::write(pid_file, child.id().to_string());

            CommandResult {
                ok: true,
                message: format!("{name} started on port {port}."),
                data: None,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CommandResult {
            ok: false,
            message: "typesense-server was not found in PATH.".to_string(),
            data: None,
        },
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not run typesense-server: {error}"),
            data: None,
        },
    }
}

#[tauri::command]
fn stop_typesense_instance(app: tauri::AppHandle, port: u16) -> CommandResult<()> {
    let data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("service-desk"))
        .join(format!("typesense-{port}"));
    let pid_file = data_dir.join("typesense.pid");

    let pid = match fs::read_to_string(&pid_file) {
        Ok(pid) => pid,
        Err(_) => match find_listening_pid(port) {
            Some(pid) => pid,
            None => {
                return CommandResult {
                    ok: true,
                    message: format!("No Typesense process is listening on port {port}."),
                    data: None,
                };
            }
        },
    };

    let output = Command::new("kill").arg(pid.trim()).output();

    match output {
        Ok(output) if output.status.success() => {
            let _ = fs::remove_file(pid_file);

            CommandResult {
                ok: true,
                message: format!("Typesense on port {port} stopped."),
                data: None,
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CommandResult {
                ok: false,
                message: format!("kill failed: {}", stderr.trim()),
                data: None,
            }
        }
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not stop Typesense: {error}"),
            data: None,
        },
    }
}

fn find_listening_pid(port: u16) -> Option<String> {
    let output = Command::new(resolve_binary("lsof"))
        .args([
            "-nP",
            &format!("-tiTCP:{port}"),
            "-sTCP:LISTEN",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|pid| !pid.is_empty())
        .map(ToOwned::to_owned)
}

#[tauri::command]
fn discover_processes() -> Vec<ProcessInfo> {
    let output = Command::new(resolve_binary("lsof"))
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();

    for line in stdout.lines().skip(1) {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() < 9 {
            continue;
        }

        let Ok(pid) = columns[1].parse::<u32>() else {
            continue;
        };

        push_unique_process(
            &mut processes,
            ProcessInfo {
                pid,
                name: columns[0].to_string(),
                detail: columns[8..].join(" "),
                port: extract_port(line),
            },
        );
    }

    discover_mysqld_process_infos(&mut processes);
    processes.sort_by(|left, right| left.name.cmp(&right.name).then(left.pid.cmp(&right.pid)));
    processes.truncate(100);
    processes
}

fn discover_mysqld_process_infos(processes: &mut Vec<ProcessInfo>) {
    let output = Command::new("/bin/ps").args(["aux"]).output();
    let Ok(output) = output else {
        return;
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if !line.contains("mysqld") || line.contains("rg -i") {
            continue;
        }

        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() < 2 {
            continue;
        }

        let Ok(pid) = columns[1].parse::<u32>() else {
            continue;
        };
        let port = extract_process_arg(line, "--port=").and_then(|value| value.parse::<u16>().ok());

        push_unique_process(
            processes,
            ProcessInfo {
                pid,
                name: "mysqld".to_string(),
                detail: line.to_string(),
                port,
            },
        );
    }
}

fn push_unique_process(processes: &mut Vec<ProcessInfo>, process: ProcessInfo) {
    let exists = processes
        .iter()
        .any(|existing| existing.pid == process.pid && existing.port == process.port);

    if !exists {
        processes.push(process);
    }
}

#[tauri::command]
fn stop_process(pid: u32) -> CommandResult<()> {
    let result = Command::new("/bin/kill").arg(pid.to_string()).output();

    match command_status_result(result, format!("Process {pid} stopped."), "Could not stop process") {
        CommandResult { ok: true, .. } => CommandResult {
            ok: true,
            message: format!("Process {pid} stopped."),
            data: None,
        },
        _ => {
            let admin = run_admin_shell(&format!("/bin/kill {pid}"));
            command_status_result(admin, format!("Process {pid} stopped."), "Could not stop process")
        }
    }
}

#[tauri::command]
fn start_mysql_instance(app: tauri::AppHandle, port: u16, name: String) -> CommandResult<()> {
    let managed_dir = managed_service_dir(&app, "mysql", port);
    if managed_dir.join("mysql").exists() {
        if mysql_port_running(port) {
            return CommandResult {
                ok: true,
                message: format!("{name} is already running on port {port}."),
                data: None,
            };
        }

        let mysqld = resolve_binary("mysqld");
        if !std::path::Path::new(&mysqld).exists() {
            return CommandResult {
                ok: false,
                message: "mysqld was not found in PATH.".to_string(),
                data: None,
            };
        }

        use std::os::unix::process::CommandExt;
        let socket = managed_dir.join("mysql.sock");
        let pid_file = managed_dir.join("mysqld.pid");
        let err_log = managed_dir.join("error.log");
        let mut cmd = Command::new(&mysqld);
        cmd.arg("--no-defaults");
        if let Some(basedir) = mysql_basedir(&mysqld) {
            cmd.arg(format!("--basedir={}", basedir.display()));
        }
        cmd.args([
            format!("--port={port}"),
            format!("--datadir={}", managed_dir.display()),
            format!("--socket={}", socket.display()),
            format!("--pid-file={}", pid_file.display()),
            format!("--log-error={}", err_log.display()),
        ]);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.process_group(0);

        match cmd.spawn() {
            Ok(child) => {
                drop(child);
                return wait_for_mysql_state(port, true, &format!("{name} started."));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return CommandResult {
                    ok: false,
                    message: "mysqld was not found in PATH.".to_string(),
                    data: None,
                };
            }
            Err(error) => {
                return CommandResult {
                    ok: false,
                    message: format!("Could not run mysqld: {error}"),
                    data: None,
                };
            }
        }
    }

    if port == 3308 && std::path::Path::new("/Applications/XAMPP/xamppfiles/xampp").exists() {
        let result = run_xampp_admin_command("startmysql");
        return match command_status_result(result, format!("{name} start command sent."), "Could not start MySQL") {
            CommandResult { ok: true, .. } => wait_for_mysql_state(port, true, &format!("{name} started.")),
            error => error,
        };
    }

    let result =
        Command::new("mysql.server").arg("start").output().or_else(|_| {
            Command::new("brew")
                .args(["services", "start", "mysql"])
                .output()
        });

    match command_status_result(result, format!("{name} start command sent."), "Could not start MySQL") {
        CommandResult { ok: true, .. } => wait_for_mysql_state(port, true, &format!("{name} started.")),
        error => error,
    }
}

#[tauri::command]
fn stop_mysql_instance(app: tauri::AppHandle, port: u16, name: String) -> CommandResult<()> {
    let managed_dir = managed_service_dir(&app, "mysql", port);
    if managed_dir.join("mysql").exists() {
        let socket = managed_dir.join("mysql.sock");
        let pid_file = managed_dir.join("mysqld.pid");

        let mysqladmin = resolve_binary("mysqladmin");
        if std::path::Path::new(&mysqladmin).exists() && socket.exists() {
            let _ = Command::new(&mysqladmin)
                .args([
                    "--socket".to_string(),
                    socket.to_string_lossy().into_owned(),
                    "shutdown".to_string(),
                ])
                .output();
        }

        if mysql_port_running(port) {
            if let Ok(pid_str) = fs::read_to_string(&pid_file) {
                let trimmed = pid_str.trim();
                if !trimmed.is_empty() {
                    let _ = Command::new("/bin/kill").arg(trimmed).output();
                }
            }
        }

        return wait_for_mysql_state(port, false, &format!("{name} stopped."));
    }

    if port == 3308 && std::path::Path::new("/Applications/XAMPP/xamppfiles/xampp").exists() {
        let result = run_xampp_admin_command("stopmysql");
        return match command_status_result(result, format!("{name} stop command sent."), "Could not stop MySQL") {
            CommandResult { ok: true, .. } => wait_for_mysql_state(port, false, &format!("{name} stopped.")),
            error => error,
        };
    }

    let result =
        Command::new("mysql.server").arg("stop").output().or_else(|_| {
            Command::new("brew")
                .args(["services", "stop", "mysql"])
                .output()
        });

    match command_status_result(result, format!("{name} stop command sent."), "Could not stop MySQL") {
        CommandResult { ok: true, .. } => wait_for_mysql_state(port, false, &format!("{name} stopped.")),
        error => error,
    }
}

fn wait_for_mysql_state(port: u16, expected_running: bool, success_message: &str) -> CommandResult<()> {
    for _ in 0..60 {
        if mysql_port_running(port) == expected_running {
            return CommandResult {
                ok: true,
                message: success_message.to_string(),
                data: None,
            };
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    CommandResult {
        ok: false,
        message: if expected_running {
            format!("MySQL command was sent, but port {port} did not start.")
        } else {
            format!("MySQL command was sent, but port {port} is still running.")
        },
        data: None,
    }
}

fn mysql_port_running(port: u16) -> bool {
    if port == 3308 {
        return mysqladmin_socket_ping("/Applications/XAMPP/xamppfiles/var/mysql/mysql.sock")
            || mysqld_process_running(port);
    }

    tcp_reachable("127.0.0.1", port, Duration::from_millis(250)).is_ok()
        || mysqld_process_running(port)
}

fn mysqld_process_running(port: u16) -> bool {
    let output = Command::new("/bin/ps").args(["aux"]).output();
    let Ok(output) = output else {
        return false;
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.contains("mysqld") && line.contains(&format!("--port={port}")))
}

#[tauri::command]
fn unlock_xampp_admin() -> CommandResult<()> {
    if xampp_broker_available() {
        let _ = send_xampp_admin_command("quit");
        std::thread::sleep(Duration::from_millis(250));
    }

    let Ok((uid, gid)) = current_uid_gid() else {
        return CommandResult {
            ok: false,
            message: "Could not determine current user id.".to_string(),
            data: None,
        };
    };

    let script = format!(
        r#"#!/bin/sh
PIPE="/tmp/service-desk-xampp-admin.pipe"
PID="/tmp/service-desk-xampp-admin.pid"
rm -f "$PIPE"
mkfifo "$PIPE"
chown {uid}:{gid} "$PIPE"
chmod 600 "$PIPE"
echo $$ > "$PID"
chown {uid}:{gid} "$PID"
while true; do
  if read cmd < "$PIPE"; then
    case "$cmd" in
      startmysql) /Applications/XAMPP/xamppfiles/xampp startmysql ;;
      stopmysql)
        /Applications/XAMPP/xamppfiles/xampp stopmysql
        /usr/bin/pkill -f '/Applications/XAMPP/xamppfiles/bin/mysqld_safe.*var/mysql' 2>/dev/null || true
        /usr/bin/pkill -f '/Applications/XAMPP/xamppfiles/sbin/mysqld.*--port=3308' 2>/dev/null || true
        ;;
      quit) rm -f "$PIPE" "$PID"; exit 0 ;;
    esac
  fi
done
"#
    );
    let script_path = "/tmp/service-desk-xampp-admin.sh";

    if let Err(error) = fs::write(script_path, script) {
        return CommandResult {
            ok: false,
            message: format!("Could not write XAMPP helper script: {error}"),
            data: None,
        };
    }

    let result = run_admin_shell(&format!(
        "/bin/sh {script_path} </dev/null >/tmp/service-desk-xampp-admin.log 2>&1 &"
    ));

    match result {
        Ok(output) if output.status.success() => CommandResult {
            ok: true,
            message: "XAMPP admin helper unlocked for this session.".to_string(),
            data: None,
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CommandResult {
                ok: false,
                message: format!("Could not unlock XAMPP helper: {}", stderr.trim()),
                data: None,
            }
        }
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not unlock XAMPP helper: {error}"),
            data: None,
        },
    }
}

fn run_xampp_admin_command(command: &str) -> std::io::Result<std::process::Output> {
    if xampp_broker_available() {
        send_xampp_admin_command(command)?;

        return Command::new("/usr/bin/true").output();
    }

    run_admin_shell(&format!("/Applications/XAMPP/xamppfiles/xampp {command}"))
}

fn send_xampp_admin_command(command: &str) -> std::io::Result<()> {
    let mut pipe = fs::OpenOptions::new()
        .write(true)
        .open("/tmp/service-desk-xampp-admin.pipe")?;
    writeln!(pipe, "{command}")
}

fn xampp_broker_available() -> bool {
    std::path::Path::new("/tmp/service-desk-xampp-admin.pipe").exists()
}

fn current_uid_gid() -> std::io::Result<(String, String)> {
    let uid = Command::new("/usr/bin/id").arg("-u").output()?;
    let gid = Command::new("/usr/bin/id").arg("-g").output()?;

    Ok((
        String::from_utf8_lossy(&uid.stdout).trim().to_string(),
        String::from_utf8_lossy(&gid.stdout).trim().to_string(),
    ))
}

fn show_main_window(app: &tauri::AppHandle) {
    let _ = app.set_activation_policy(ActivationPolicy::Accessory);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn show_main_window_under_tray(
    app: &tauri::AppHandle,
    tray_rect: Rect,
) {
    let _ = app.set_activation_policy(ActivationPolicy::Accessory);

    if let Some(window) = app.get_webview_window("main") {
        if let Ok(size) = window.outer_size() {
            let scale_factor = window.scale_factor().unwrap_or(1.0);
            let tray_position = tray_rect.position.to_physical::<f64>(scale_factor);
            let tray_size = tray_rect.size.to_physical::<f64>(scale_factor);
            let x = (tray_position.x + (tray_size.width / 2.0) - (f64::from(size.width) / 2.0))
                .round() as i32;
            let y = (tray_position.y + tray_size.height + 8.0).round() as i32;
            let _ = window.set_position(PhysicalPosition::new(x.max(0), y.max(0)));
        }

        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn rebuild_tray_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<_>> = Vec::new();
    items.push(&show);
    items.push(&hide);
    items.push(&quit);

    let menu = Menu::with_items(app, &items)?;
    let state = app.state::<AppState>();
    if let Some(tray) = state.tray.lock().expect("tray lock").as_ref() {
        tray.set_menu(Some(menu))?;
    }

    Ok(())
}

fn run_admin_shell(command: &str) -> std::io::Result<std::process::Output> {
    let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");

    Command::new("/usr/bin/osascript")
        .args([
            "-e",
            &format!("do shell script \"{escaped}\" with administrator privileges"),
        ])
        .output()
}

fn command_status_result(
    result: std::io::Result<std::process::Output>,
    success_message: String,
    failure_prefix: &str,
) -> CommandResult<()> {
    match result {
        Ok(output) if output.status.success() => CommandResult {
            ok: true,
            message: success_message,
            data: None,
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };

            CommandResult {
                ok: false,
                message: format!("{failure_prefix}: {detail}"),
                data: None,
            }
        }
        Err(error) => CommandResult {
            ok: false,
            message: format!("{failure_prefix}: {error}"),
            data: None,
        },
    }
}

#[tauri::command]
fn discover_running_services() -> Vec<DiscoveredService> {
    let output = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut services = Vec::new();

    for line in stdout.lines().skip(1) {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        let Some(command) = columns.first() else {
            continue;
        };

        let command_lower = command.to_lowercase();
        let Some(port) = extract_port(line) else {
            continue;
        };

        if command_lower.contains("redis") {
            push_unique_service(
                &mut services,
                DiscoveredService {
                    kind: "redis".to_string(),
                    name: if port == 6379 {
                        "redis".to_string()
                    } else {
                        format!("redis {port}")
                    },
                    host: "127.0.0.1".to_string(),
                    port,
                    version: "Redis".to_string(),
                    api_key: None,
                    socket: None,
                },
            );
        } else if command_lower.contains("typesense") {
            push_unique_service(
                &mut services,
                DiscoveredService {
                    kind: "typesense".to_string(),
                    name: if port == 8108 {
                        "typesense".to_string()
                    } else {
                        format!("typesense {port}")
                    },
                    host: "127.0.0.1".to_string(),
                    port,
                    version: "Typesense".to_string(),
                    api_key: Some("change-me".to_string()),
                    socket: None,
                },
            );
        } else if command_lower.contains("mysql") || command_lower.contains("mysqld") {
            push_unique_service(
                &mut services,
                DiscoveredService {
                    kind: "mysql".to_string(),
                    name: if port == 3308 {
                        "xampp mysql".to_string()
                    } else if port == 3306 {
                        "mysql".to_string()
                    } else {
                        format!("mysql {port}")
                    },
                    host: "127.0.0.1".to_string(),
                    port,
                    version: "MySQL".to_string(),
                    api_key: None,
                    socket: None,
                },
            );
        }
    }

    discover_mysqld_processes(&mut services);

    services
}

fn discover_mysqld_processes(services: &mut Vec<DiscoveredService>) {
    let output = Command::new("/bin/ps").args(["aux"]).output();
    let Ok(output) = output else {
        return;
    };

    if !output.status.success() {
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if !line.contains("mysqld") || line.contains("rg -i") {
            continue;
        }

        let socket = extract_process_arg(line, "--socket=");
        let port = extract_process_arg(line, "--port=")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or_else(|| {
                if line.contains("/Applications/XAMPP/") {
                    3308
                } else {
                    3306
                }
            });

        push_unique_service(
            services,
            DiscoveredService {
                kind: "mysql".to_string(),
                name: if line.contains("/Applications/XAMPP/") {
                    "xampp mysql".to_string()
                } else if port == 3306 {
                    "mysql".to_string()
                } else {
                    format!("mysql {port}")
                },
                host: "127.0.0.1".to_string(),
                port,
                version: "MySQL".to_string(),
                api_key: None,
                socket,
            },
        );
    }
}

fn extract_process_arg(line: &str, prefix: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(prefix))
        .map(ToOwned::to_owned)
}

fn push_unique_service(services: &mut Vec<DiscoveredService>, service: DiscoveredService) {
    let exists = services
        .iter()
        .any(|existing| existing.kind == service.kind && existing.port == service.port);

    if !exists {
        services.push(service);
    }
}

fn extract_port(line: &str) -> Option<u16> {
    let endpoint = line
        .split_whitespace()
        .find(|part| part.contains("TCP") && part.contains(':'))
        .or_else(|| line.split_whitespace().rev().find(|part| part.contains(':')))?;
    let port_text = endpoint.rsplit(':').next()?;
    let digits = port_text
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();

    digits.parse::<u16>().ok()
}

#[tauri::command]
fn check_typesense_health(config: TypesenseConfig) -> CommandResult<()> {
    let url = service_url(&config, "/health");

    match request_json(&url, &config.api_key) {
        Ok(value) => {
            let healthy = value
                .get("ok")
                .and_then(Value::as_bool)
                .or_else(|| value.get("healthy").and_then(Value::as_bool))
                .unwrap_or(true);

            CommandResult {
                ok: healthy,
                message: if healthy {
                    "Typesense health check passed.".to_string()
                } else {
                    "Typesense reported an unhealthy status.".to_string()
                },
                data: None,
            }
        }
        Err(error) => CommandResult {
            ok: false,
            message: format!("Typesense health check failed: {error}"),
            data: None,
        },
    }
}

#[tauri::command]
fn list_typesense_collections(config: TypesenseConfig) -> CommandResult<CollectionsPayload> {
    let url = service_url(&config, "/collections");

    match request_json(&url, &config.api_key) {
        Ok(Value::Array(items)) => {
            let collections = items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();

            CommandResult {
                ok: true,
                message: format!("Loaded {} collection(s).", collections.len()),
                data: Some(CollectionsPayload { collections }),
            }
        }
        Ok(_) => CommandResult {
            ok: false,
            message: "Typesense returned an unexpected collections response.".to_string(),
            data: None,
        },
        Err(error) => CommandResult {
            ok: false,
            message: format!("Could not load Typesense collections: {error}"),
            data: None,
        },
    }
}

fn service_url(config: &TypesenseConfig, path: &str) -> String {
    let protocol = if config.protocol == "https" {
        "https"
    } else {
        "http"
    };
    format!("{protocol}://{}:{}{path}", config.host, config.port)
}

fn request_json(url: &str, api_key: &str) -> Result<Value, String> {
    let output = Command::new(resolve_binary("curl"))
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "5",
            "-H",
            &format!("X-TYPESENSE-API-KEY: {api_key}"),
            url,
        ])
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "curl was not found in PATH.".to_string()
            } else {
                format!("Could not run curl: {error}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl failed: {}", stderr.trim()));
    }

    serde_json::from_slice::<Value>(&output.stdout).map_err(|error| error.to_string())
}

fn resolve_brew() -> Option<String> {
    for candidate in ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn ensure_mysql_installed() -> Result<(), String> {
    let Some(brew) = resolve_brew() else {
        return Err(
            "Homebrew is not installed. Install Homebrew first (https://brew.sh) or install MySQL manually."
                .to_string(),
        );
    };

    let output = Command::new(&brew)
        .args(["install", "mysql"])
        .output()
        .map_err(|error| format!("Could not run brew install mysql: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}\n{}", stderr.trim(), stdout.trim());
    if combined.to_lowercase().contains("already installed") {
        return Ok(());
    }
    Err(format!("brew install mysql failed: {}", combined.trim()))
}

fn resolve_binary(name: &str) -> String {
    let candidates = [
        format!("/Applications/XAMPP/xamppfiles/bin/{name}"),
        format!("/Applications/XAMPP/xamppfiles/sbin/{name}"),
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("/usr/bin/{name}"),
        format!("/usr/sbin/{name}"),
        format!("/bin/{name}"),
    ];

    for candidate in candidates {
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }

    if let Some(path) = resolve_dbngin_binary(name) {
        return path;
    }

    if let Ok(output) = Command::new("/bin/zsh")
        .args(["-lc", &format!("command -v {name}")])
        .output()
    {
        if output.status.success() {
            if let Some(path) = String::from_utf8_lossy(&output.stdout).lines().next() {
                let path = path.trim();
                if !path.is_empty() {
                    return path.to_string();
                }
            }
        }
    }

    name.to_string()
}

fn resolve_dbngin_binary(name: &str) -> Option<String> {
    let service = if name.starts_with("redis") {
        "redis"
    } else if name.starts_with("mysql") {
        "mysql"
    } else if name.starts_with("typesense") {
        "typesense"
    } else {
        return None;
    };

    let service_dir = std::path::Path::new("/Users/Shared/DBngin").join(service);
    let entries = fs::read_dir(service_dir).ok()?;
    let mut matches = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("bin").join(name))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();

    matches.sort();
    matches
        .pop()
        .map(|path| path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(false) = event {
                if window.label() == "main" {
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let _ = app.set_activation_policy(ActivationPolicy::Accessory);
            let icon = app
                .default_window_icon()
                .expect("default window icon is required for tray")
                .clone();

            let tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Service Desk")
                .icon(icon)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "hide" => {
                        hide_main_window(app)
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            match window.is_visible() {
                                Ok(true) => {
                                    let _ = window.hide();
                                }
                                _ => show_main_window_under_tray(&tray.app_handle(), rect),
                            }
                        }
                    }
                })
                .build(app)?;
            let state = app.state::<AppState>();
            *state.tray.lock().expect("tray lock") = Some(tray);
            rebuild_tray_menu(&app.handle())?;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            test_redis_connection,
            test_tcp_connection,
            test_mysql_connection,
            infer_mysql_port_from_socket,
            list_redis_keys,
            create_managed_service,
            start_redis_instance,
            stop_redis_instance,
            delete_managed_service,
            test_typesense_connection,
            start_typesense_instance,
            stop_typesense_instance,
            discover_running_services,
            start_mysql_instance,
            stop_mysql_instance,
            unlock_xampp_admin,
            discover_processes,
            stop_process,
            check_typesense_health,
            list_typesense_collections,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
