# Service Desk

A lightweight macOS desktop app for managing local developer services such as Redis, MySQL/MariaDB, and Typesense.

Service Desk runs from the macOS menu bar and gives you a compact control surface for starting, stopping, checking, and removing local service instances.

## Features

- Manage Redis, MySQL/MariaDB, and Typesense instances from one desktop app.
- Create managed local instances with per-port data directories.
- Discover and control existing local service processes.
- Start and stop XAMPP MySQL through the app helper when needed.
- Select and copy status messages and error output from the bottom status bar.

## Requirements

- macOS
- Node.js and npm
- Rust and Cargo
- One or more supported service binaries:
  - `redis-server` and `redis-cli` for Redis
  - `mysqld`, `mysqladmin`, and either `mysql_install_db` or `mariadb-install-db` for MySQL/MariaDB
  - `typesense-server` for Typesense

The app looks for common Homebrew, system, XAMPP, and DBngin install paths. If MySQL is missing and Homebrew is available, the backend can attempt to install MySQL with `brew install mysql`.

## MySQL and XAMPP

Managed MySQL instances use their own data directory under the app data folder. When the available server is XAMPP/MariaDB, Service Desk initializes the instance with `mariadb-install-db` or `mysql_install_db` and starts `mysqld` with `--no-defaults` so the managed instance does not reuse XAMPP's global config or data directory.

## Development

```bash
npm install
npm run tauri dev
```

## Scripts

```bash
npm run dev        # Frontend only
npm run build      # TypeScript + Vite build
npm run tauri dev  # Desktop app in development
```
