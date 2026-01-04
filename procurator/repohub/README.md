# Repohub - Project & Repository Management Platform

A web-based platform for managing projects and their associated repositories, built with Rust, Axum, and SQLite.

## Features

✅ **User Management** - Create and manage users
✅ **Project Organization** - Group repositories into projects (like organizations)
✅ **Repository Tracking** - Track multiple Git repositories per project
✅ **Clean UI** - Modern, GitHub-inspired interface using Askama templates
🚧 **Configuration Management** - Infrastructure and deployment settings (planned)
🚧 **E2E Testing** - Cross-service testing framework (planned)
🚧 **Nix Flake Integration** - Parse and display flake configurations (planned)
🚧 **Build Integration** - CI/CD build tracking (planned)

## Architecture

Following a hexagonal architecture approach:

- **Database Layer** (`database.rs`) - SQLite with sqlx, handles all data persistence
- **Domain Layer** (`models.rs`) - Core business entities (User, Project, Repository)
- **Web Layer** (`web.rs`) - HTTP handlers with Askama HTML templates
- **Configuration** (`config.rs`) - Application settings

## Quick Start

### Build

```bash
cd procurator/repohub
cargo build
```

### Run

```bash
cargo run
```

The server will start on `http://localhost:3001`

### Test the API

You can create entities using curl:

```bash
# Create a user
curl -X POST http://localhost:3001/users \
  -H "Content-Type: application/json" \
  -d '{"username": "alice", "email": "alice@example.com"}'

# Create a project
curl -X POST http://localhost:3001/alice/projects \
  -H "Content-Type: application/json" \
  -d '{"name": "awesome-project", "description": "My awesome microservices"}'

# Add a repository
curl -X POST http://localhost:3001/alice/awesome-project/repositories \
  -H "Content-Type: application/json" \
  -d '{"name": "api-service", "git_url": "git@github.com:alice/api-service.git"}'
```

Then visit the web interface:

- http://localhost:3001/ - List all users
- http://localhost:3001/alice - View user and their projects
- http://localhost:3001/alice/awesome-project - View project and repositories
- http://localhost:3001/alice/awesome-project/api-service - View repository details

## Database Schema

### Users
- `id` - Primary key
- `username` - Unique username
- `email` - Optional email
- `created_at` - Timestamp

### Projects
- `id` - Primary key
- `name` - Project name
- `owner_id` - Foreign key to users
- `description` - Optional description
- `created_at` - Timestamp
- Unique constraint on (owner_id, name)

### Repositories
- `id` - Primary key
- `project_id` - Foreign key to projects
- `name` - Repository name
- `git_url` - Git URL
- `created_at` - Timestamp
- Unique constraint on (project_id, name)

### Project Members (for collaboration)
- `project_id` - Foreign key to projects
- `user_id` - Foreign key to users
- `role` - Member role (owner, member, etc.)
- `created_at` - Timestamp

## Project Structure

```
repohub/
├── src/
│   ├── config.rs        # Configuration
│   ├── database.rs      # Database layer
│   ├── models.rs        # Domain models
│   ├── web.rs           # Web handlers
│   ├── lib.rs           # Library exports
│   └── main.rs          # Application entry point
├── templates/
│   ├── base.html        # Base template with CSS
│   ├── index.html       # Users list
│   ├── user.html        # User profile with projects
│   ├── project.html     # Project with repositories
│   ├── repository.html  # Repository details
│   └── not_implemented.html  # Placeholder for future features
├── Cargo.toml
└── README.md
```

## Next Steps

The project is set up with a solid foundation. Future development will focus on:

1. **Configuration Management** - Auto-generated repository with infrastructure settings
2. **Nix Flake Integration** - Parse flake.nix files and display metadata
3. **CI/CD Integration** - Connect with the CI service for build tracking
4. **Testing Framework** - E2E testing across all services in a project
5. **Real Git Integration** - Clone, browse, and manage actual Git repositories
6. **Collaboration** - Multi-user project access with permissions
7. **Search & Filtering** - Find projects and repositories easily

## Design Philosophy

- **Simplistic** - Clean, straightforward code following the same patterns as `ci_service`
- **Hexagonal Architecture** - Clear separation between database, domain, and web layers
- **Type Safety** - Leverage Rust's type system for correctness
- **Developer Experience** - Fast compilation, clear error messages, easy to extend
