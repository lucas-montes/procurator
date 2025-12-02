# CI Service Architecture & Logging Reference

## Project Structure

```
procurator/ci_service/
├── Cargo.toml                    # Dependencies & metadata
├── post-receive                  # Git hook script (embedded)
├── src/
│   ├── main.rs                  # Application entry point & routing
│   ├── worker.rs                # Build executor (polls queue, runs nix)
│   ├── api.rs                   # HTTP API handlers (POST /api/builds)
│   ├── queue.rs                 # SQLite build queue management
│   ├── config.rs                # Configuration management (OnceLock)
│   ├── error.rs                 # Domain-specific error types
│   ├── nix_parser.rs            # Nix flake metadata parser
│   ├── git_url.rs               # Git URL builder for Nix
│   ├── repo_manager.rs          # Bare repo lifecycle management
│   └── web.rs                   # Web UI & API handlers
├── static/
│   └── index.html               # Single Page Application
├── LOGGING_CHANGES.md           # Detailed changelog
├── LOGGING_STYLE_GUIDE.md       # Style guide for contributors
└── REFACTORING_SUMMARY.md       # Summary of changes
```

## Data Flow

### 1. Git Push → Build Queue
```
Developer pushes → Git server
                      ↓
                  post-receive hook
                      ↓
                  HTTP POST /api/builds
                      ↓
                  api::create_build()
                      ↓
                  queue.enqueue()
                      ↓
                  SQLite INSERT
```

### 2. Queue → Worker Processing
```
Worker polls queue (every 2-5 seconds)
        ↓
queue.get_pending() returns Build
        ↓
worker.process_build()
        ↓
nix flake check {git_url}
        ↓
Capture stdout/stderr
        ↓
queue.set_logs()
        ↓
queue.update_status(Success|Failed)
```

### 3. Web UI → Real-time Updates
```
Browser fetches GET /api/builds
        ↓
web::list_builds()
        ↓
Polls every 5 seconds
        ↓
Updates UI with latest status
```

## Module Responsibilities

### main.rs
- **Starts the application**
- Initializes configuration
- Sets up tracing/logging
- Creates AppState (queue + repo_manager)
- Spawns background worker
- Defines HTTP routes
- Starts Axum server

### worker.rs
- **Processes builds from queue**
- Polls `queue.get_pending()` every 2-5 seconds
- Executes `nix flake check {git_url}`
- Captures build output
- Stores logs in database
- Updates build status
- Implements retry logic (max 3 attempts)

### api.rs
- **Handles Git hook requests**
- `POST /api/builds` endpoint
- Extracts commit/repo/branch info
- Validates input
- Calls `queue.enqueue()`
- Returns build ID + status

### queue.rs
- **SQLite-based build queue**
- Build struct (database row)
- Status enum (Queued, Running, Success, Failed)
- Operations:
  - `enqueue()` - add new build
  - `get_pending()` - fetch for processing
  - `update_status()` - transition states
  - `set_logs()` - store build output
  - `can_retry()` - check retry eligibility
  - `increment_retry()` - bump attempt count
  - `list_all_builds()` - for web UI

### config.rs
- **Global configuration singleton**
- Uses OnceLock for thread-safe single initialization
- Fields:
  - `database_url` - SQLite file path
  - `bind_address` - HTTP listen address
  - `repos_base_path` - Git repo storage
  - `max_retries` - Failed build retries
  - `worker_poll_interval_ms` - Queue polling interval

### error.rs
- **Domain-specific error types**
- WorkerError enum:
  - Database - SQLite errors
  - Process - Command execution errors
  - Nix - nix flake check failures
  - Git - Git URL/operation errors
  - Io - File I/O errors

### nix_parser.rs
- **Extracts Nix flake metadata**
- Commands: `nix flake metadata --json`
- Parses:
  - Packages
  - Checks (tests)
  - Apps
  - Dev shells
  - NixOS modules
- Foundation for future `procurator.*` parsing

### git_url.rs
- **Builds git+file:// URLs for Nix**
- Handles absolute path resolution
- Creates URLs like: `git+file:///path/to/repo.git?rev=commitsha`
- Validates paths and commits

### repo_manager.rs
- **Manages bare Git repositories**
- Operations:
  - `create_repo()` - init bare repo
  - `install_post_receive_hook()` - embed hook script
  - `get_repo_path()` - resolve repo location
  - `delete_repo()` - remove repository (careful!)
- Post-receive hook is compiled into binary

### web.rs
- **Web UI and REST API**
- Static route: `GET /` - serves index.html
- Build API:
  - `GET /api/builds` - list all
  - `GET /api/builds/{id}` - details
  - `GET /api/builds/{id}/logs` - streaming logs
- Repo API:
  - `GET /api/repos` - list
  - `POST /api/repos` - create
  - `GET /api/repos/{name}` - details
- Real-time:
  - `GET /api/events` - SSE stream

## Logging Patterns by Component

### worker.rs
```rust
// Startup
info!(target: "procurator::worker", "Worker started");

// Processing starts
info!(build_id = id, repo = name, branch = branch, "Starting build processing");

// Build succeeded
info!(build_id = id, "Build succeeded");

// Build failed
error!(build_id = id, exit_code = code, "Build failed");

// Retry scheduled
info!(build_id = id, attempt = n, "Scheduling retry for build");
```

### api.rs
```rust
// Request received
info!(
    repo = repo,
    branch = branch,
    commit = commit_short,
    author = author,
    "Build request received"
);

// Build enqueued
info!(
    build_id = id,
    repo = repo,
    branch = branch,
    "Build enqueued successfully"
);

// Enqueue failed
error!(
    repo = repo,
    branch = branch,
    error = error_msg,
    "Failed to enqueue build"
);
```

### main.rs
```rust
// Startup
info!(
    database = url,
    bind_address = addr,
    repos_path = path,
    "Starting Procurator CI Service"
);

// Worker spawned
info!(target: "procurator::main", "Build worker spawned");

// Server ready
info!(target: "procurator::main", bind_address = addr, "Starting HTTP server");
```

## Configuration

Set via environment variables:

```bash
# Database location
export DATABASE_URL="../ci.db"

# HTTP bind address
export BIND_ADDRESS="0.0.0.0:3000"

# Git repos location
export REPOS_BASE_PATH="../repos"

# Retry config
export MAX_RETRIES="3"

# Worker polling interval (ms)
export WORKER_POLL_INTERVAL_MS="2000"

# Logging level
export RUST_LOG="info"
```

## Compilation & Verification

```bash
# Check for errors
cargo check

# Build with optimizations
cargo build --release

# Run with default INFO logging
cargo run

# Run with DEBUG logging
RUST_LOG=debug cargo run

# Run specific worker logging
RUST_LOG=procurator::worker=debug cargo run

# Run tests (if any)
cargo test
```

## Database Schema

```sql
-- builds table
CREATE TABLE builds (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER,
    repo_name TEXT,
    repo_path TEXT,
    commit_hash TEXT,
    branch TEXT,
    status TEXT,  -- queued, running, success, failed
    retry_count INTEGER,
    max_retries INTEGER,
    created_at TEXT,
    started_at TEXT,
    finished_at TEXT
);

-- build_logs table
CREATE TABLE build_logs (
    id INTEGER PRIMARY KEY,
    build_id INTEGER,
    log_data TEXT
);

-- repos table
CREATE TABLE repos (
    id INTEGER PRIMARY KEY,
    name TEXT,
    path TEXT,
    description TEXT,
    created_at TEXT
);
```

## Future Enhancements

### Immediate
1. JSON logging formatter for aggregation
2. Trace IDs for request tracing
3. Build metrics (duration, success rate)
4. Distributed tracing (OpenTelemetry)

### Short-term
1. Control plane integration (Cap'n Proto RPC)
2. Procurator flake schema parsing
3. Deployment triggering on successful builds
4. Web UI improvements (websockets, live updates)

### Medium-term
1. Secret management integration
2. Multi-worker support (distribute builds)
3. Build caching
4. Performance optimizations

## Key Concepts

### Build Status Lifecycle
```
Queued
  ↓
Running
  ↓
Success (or Failed)
  ↓
Archived (optional cleanup)
```

### Retry Logic
- Max 3 attempts by default (configurable)
- On failure, check `can_retry()`
- Increment `retry_count`
- Re-enqueue as Queued
- After max_retries exhausted, mark as Failed

### Git Integration
- Post-receive hook is executed on every push
- Sends HTTP POST with repo/commit/branch info
- API enqueues build for processing
- No direct Git interaction after hook

### Nix Integration
- Uses `nix flake check {git_url}` to validate flakes
- URL format: `git+file:///absolute/path?rev=commitsha`
- Captures all nix output and errors
- Stores in database for UI display

## Troubleshooting

### Build Queue Empty?
- Check if post-receive hook is executable
- Verify API endpoint is accessible
- Check database permissions

### Worker Not Processing?
- Check if worker background task is running
- Verify database can be read
- Check nix is installed and working

### Builds Always Fail?
- Verify git_url is correct
- Check nix flake check succeeds manually
- Review build logs in database

## Performance Characteristics

- **Queue polling**: 2-5 second intervals (configurable)
- **Database**: SQLite (single file, local access)
- **Concurrent builds**: Limited by worker availability
- **Log storage**: All stdout/stderr captured in database

## Limitations (Current)

- Single-threaded worker (processes one build at a time)
- SQLite (not suitable for 10k+ builds without sharding)
- No authentication on API
- No multi-node support
- Logs stored in database (could grow large)

## See Also

- `LOGGING_CHANGES.md` - Detailed refactoring notes
- `LOGGING_STYLE_GUIDE.md` - Logging best practices
- `REFACTORING_SUMMARY.md` - High-level summary
