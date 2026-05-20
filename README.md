# Sentinel Proxy

<p align="center">
  <img src="https://img.shields.io/badge/Rust-1.80+-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/Web-Axum%20%2B%20Tokio-46E0B6?style=for-the-badge" alt="Web Framework">
  <img src="https://img.shields.io/badge/Proxy-Round--Robin%20Load%20Balancing-FF6B6B?style=for-the-badge" alt="Reverse Proxy">
</p>

<p align="center">
  Rust reverse proxy with round-robin upstream routing, forwarding headers, health checks, request stats, and structured logging.
</p>

Sentinel Proxy is a small Rust reverse proxy built with Axum, Tokio, and Reqwest. It forwards incoming HTTP requests to one or more configured upstream services using round-robin routing and exposes lightweight operational endpoints for health and runtime stats.

## Features

- Reverse proxy for all non-admin routes.
- Round-robin load balancing across `BACKEND_TARGETS`.
- Adds `X-Forwarded-For`, `X-Forwarded-Proto`, and `X-Forwarded-Host` for upstream services.
- Adds `X-Sentinel-Upstream` to proxy responses so you can see which backend handled the request.
- `/health` endpoint for liveness checks.
- `/admin/stats` endpoint for request counters, targets, and uptime.
- Structured logs through `tracing`.
- Environment-based configuration with optional `.env` support.
- Graceful shutdown on `Ctrl+C`.

## Project Structure

```text
sentinel-proxy/
|-- Cargo.toml
|-- Cargo.lock
|-- .env.example
|-- README.md
`-- src/
    `-- main.rs
```

## Prerequisites

Install Rust from [rustup.rs](https://rustup.rs/), then confirm the tools are available:

```powershell
rustc --version
cargo --version
```

## Configuration

Create a local `.env` file from the example:

```powershell
Copy-Item .env.example .env
```

Example configuration:

```env
PORT=3000
BACKEND_TARGETS="https://jsonplaceholder.typicode.com,https://dummyjson.com"
RUST_LOG=info
```

Configuration options:

| Variable | Default | Description |
| --- | --- | --- |
| `PORT` | `3000` | Port the proxy listens on. |
| `BACKEND_TARGETS` | `https://jsonplaceholder.typicode.com` | Comma-separated upstream base URLs. |
| `RUST_LOG` | `info` | Log filter used by `tracing-subscriber`. |

## Run Locally

Start the proxy:

```powershell
cargo run
```

The service listens on:

```text
http://localhost:3000
```

## Build

Create a debug build:

```powershell
cargo build
```

Create an optimized release build:

```powershell
cargo build --release
```

Run the release binary:

```powershell
.\target\release\sentinel-proxy.exe
```

On Linux or macOS:

```bash
./target/release/sentinel-proxy
```

## Test And Verify

Format the code:

```powershell
cargo fmt
```

Compile-check without producing a final binary:

```powershell
cargo check
```

Run tests:

```powershell
cargo test
```

Build the project:

```powershell
cargo build
```

Start the proxy:

```powershell
cargo run
```

In another terminal, verify health:

```powershell
Invoke-RestMethod http://localhost:3000/health
```

Expected response:

```json
{
  "status": "ok"
}
```

Verify stats:

```powershell
Invoke-RestMethod http://localhost:3000/admin/stats
```

Example response:

```json
{
  "total_requests": 0,
  "successful_requests": 0,
  "failed_requests": 0,
  "last_target_index": 0,
  "targets": ["https://jsonplaceholder.typicode.com"],
  "uptime_seconds": 5
}
```

Verify proxying:

```powershell
Invoke-RestMethod http://localhost:3000/posts/1
```

If the default upstream is used, this should return a JSONPlaceholder post.

Verify forwarded headers and upstream selection:

```powershell
$response = Invoke-WebRequest http://localhost:3000/posts/1
$response.Headers["X-Sentinel-Upstream"]
```

The response should include an `X-Sentinel-Upstream` header showing the backend URL selected by round-robin routing.

## Endpoints

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/health` | Returns service health. |
| `GET` | `/admin/stats` | Returns runtime stats. |
| Any | `/*` | Proxies request to the next configured upstream. |

## Proxy Headers

Sentinel Proxy forwards these request headers to upstream services:

- `X-Forwarded-For`: Appends the client IP to the forwarding chain.
- `X-Forwarded-Proto`: Set to `http` by the proxy when not already present.
- `X-Forwarded-Host`: Derived from the incoming `Host` header when not already present.

Sentinel Proxy also adds this response header:

- `X-Sentinel-Upstream`: The configured backend base URL that handled the proxied request.

## Push To GitHub

This repository can be pushed to:

```text
https://github.com/MihirMohapatra/sentinel-proxy
```

Initialize git if needed:

```powershell
git init
git branch -M main
```

Add the GitHub remote:

```powershell
git remote add origin https://github.com/MihirMohapatra/sentinel-proxy.git
```

Check what will be committed:

```powershell
git status
```

Commit the project:

```powershell
git add Cargo.toml Cargo.lock .env.example .gitignore README.md src/main.rs
git commit -m "Initial Rust reverse proxy"
```

Push to GitHub:

```powershell
git push -u origin main
```

If the remote already exists locally, update it instead:

```powershell
git remote set-url origin https://github.com/MihirMohapatra/sentinel-proxy.git
```

## Development Notes

- Do not commit `.env`; keep secrets and local configuration out of git.
- `target/` is generated by Cargo and should not be committed.
- `Cargo.lock` should be committed for this binary application so builds are reproducible.
- Run `cargo fmt`, `cargo check`, and `cargo test` before pushing changes.
- Feature work is easiest to review when developed on a short-lived branch and merged back to `main`.
