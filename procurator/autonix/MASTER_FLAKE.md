# Master Flake - Central Orchestration (Future Work)

## Overview
The "master flake" concept: a central flake that aggregates multiple project flakes for orchestration across a microservices architecture or monorepo collection.

## Key Questions & Ideas

### 1. Master Flake Structure
**Question:** Should procurator auto-generate the master flake, or does the user create it manually?

**Options:**
- **A) User-created template:** Procurator provides a template, user fills in project paths
- **B) Auto-generated:** `procurator init --master` scans directories and generates master flake
- **C) Hybrid:** User declares projects in a config file, procurator generates the flake

**Considerations:**
- Discovery: How does master flake find project flakes? File system scan? Config file?
- Updates: When a project flake changes, how does master flake know?

### 2. Service Configuration Extraction
**Question:** Where do port/health_check/env_vars come from?

**Sources:**
- Docker-compose ports mapping (`ports: ["5432:5432"]`)
- Hardcoded defaults (postgres=5432, redis=6379)
- CI/CD service definitions (GitHub Actions services)
- User annotation/config file (`procurator.toml`)

**Proposed approach:**
1. Extract from docker-compose if available
2. Fall back to standard ports (postgres:5432, redis:6379)
3. Allow user override via config

### 3. Dependency Graph Tracking
**Question:** What should the dependency graph track?

**Options:**
- **Services only:** postgres → api → frontend
- **Packages too:** myapp-core → myapp-api (internal deps)
- **External deps:** this-repo → other-repo (cross-project deps)
- **All of the above**

**Use cases:**
- Visualization: See full architecture at a glance
- Startup order: Launch services in correct dependency order
- Impact analysis: "If I change service X, what breaks?"
- Deployment planning: Deploy in correct order

**Proposed tracking:**
```rust
struct DependencyGraph {
    // Service dependencies (runtime)
    service_deps: Vec<ServiceDependency>,  // api depends_on: [postgres, redis]

    // Package dependencies (build-time, internal)
    package_deps: Vec<PackageDependency>,  // myapp-api depends on myapp-core

    // External dependencies (cross-repo)
    external_deps: Vec<ExternalDependency>, // project-a depends on project-b's API
}
```

### 4. Service Manager Implementation
**Question:** How should the custom service manager work?

**Options:**
- **A) Shell script in flake:** `shellHook` starts services
- **B) Separate binary:** `procurator-services start`
- **C) Nix app:** `nix run .#services`
- **D) Process manager:** Write our own mini process-compose

**Requirements:**
- Start services in dependency order
- Health checks to verify services are ready
- Restart on failure
- Log aggregation
- Status monitoring
- Clean shutdown

**Proposed approach:**
```
nix run .#services start     # Start all services
nix run .#services stop      # Stop all services
nix run .#services status    # Show service status
nix run .#services logs <service>  # View logs
```

### 5. Multi-Project DevShells
**Question:** How to handle devShells across multiple projects?

**Options:**
- **A) One giant devShell:** All tools from all projects
  - Pro: Simple, everything available
  - Con: Slow, cluttered environment

- **B) Per-project devShells:** User switches between them
  - `nix develop .#project-a`
  - `nix develop .#project-b`
  - Pro: Clean, focused environments
  - Con: Need to switch when working across projects

- **C) Composable devShells:** Master provides composition helpers
  - `nix develop .#api-and-frontend`
  - `nix develop .#backend-stack`
  - Pro: Flexible, user chooses what to load
  - Con: Complex to implement

**Proposed:** Start with (B), add (C) later as needed

### 6. Visualization Format
**Question:** How to visualize the dependency graph?

**Options:**
- **Mermaid diagram:** Generates markdown with mermaid syntax
- **Graphviz DOT:** Traditional graph format
- **Interactive web UI:** Full featured visualization tool
- **JSON output:** Let user visualize however they want

**Proposed:**
```
nix run .#visualize          # Opens interactive web UI
nix run .#visualize --format mermaid > deps.md
nix run .#visualize --format dot > deps.dot
nix run .#visualize --format json > deps.json
```

### 7. Environment Variable Conflicts
**Question:** How to handle env var conflicts across projects?

**Scenario:**
```
project-a: DATABASE_URL=postgres://localhost/db_a
project-b: DATABASE_URL=postgres://localhost/db_b
```

**Options:**
- **A) Namespace by project:** `PROJECT_A_DATABASE_URL`, `PROJECT_B_DATABASE_URL`
- **B) Error and require user resolution**
- **C) Last one wins + warning**
- **D) Context-aware:** Each project's shell only sees its own vars

**Proposed:** Use (A) with automatic namespacing, document in generated flake

### 8. Service Version Conflicts
**Question:** Can different checks require different service versions?

**Scenario:**
```
check-a: needs postgres:15
check-b: needs postgres:14
```

**Options:**
- **A) Multiple instances:** Run both postgres:15 and postgres:14 on different ports
- **B) Choose highest version:** Use postgres:15 for both
- **C) Error and require user resolution**

**Proposed:** Start with (C) - error if conflict detected, let user resolve manually

### 9. Procfile-like Experience
**Question:** How to provide Procfile-like experience with Nix?

**Traditional Procfile:**
```
web: npm start
worker: python worker.py
redis: redis-server
```

**Nix equivalent:**
```nix
{
  processes = {
    web = { exec = "npm start"; };
    worker = { exec = "python worker.py"; };
  };

  services = {
    redis.enable = true;
  };
}
```

**Master flake aggregation:**
```nix
{
  # Combines all project processes and services
  apps.up = {
    type = "app";
    program = "${serviceManager}/bin/procurator-services up";
  };
}
```

**Commands:**
```
nix run .#up              # Start everything (all projects)
nix run .#up -- api       # Start only api project
nix run .#up -- api frontend  # Start api and frontend
```

### 10. Implementation Priority

**Phase 1 (Current):** Project-level flakes
- ✅ Analysis → FlakeConfig
- ✅ Generate flake.nix with packages, devShells, checks
- ✅ Custom `procurator` output with services metadata

**Phase 2:** Master flake basics
- [ ] Master flake generation from multiple project flakes
- [ ] Per-project devShells
- [ ] Service dependency graph
- [ ] Basic visualization (JSON output)

**Phase 3:** Advanced orchestration
- [ ] Custom service manager
- [ ] Health checks and auto-restart
- [ ] Interactive visualization
- [ ] Composable devShells

**Phase 4:** Production features
- [ ] Deployment coordination
- [ ] Cross-project dependencies
- [ ] Impact analysis
- [ ] CI/CD integration

## Related Files
- `src/repo/analysis.rs` - Analysis structures with Services
- `src/repo/flake.rs` - FlakeConfig (to be implemented)
- Future: `src/master/` - Master flake generation module

## References
- devenv: Uses custom `outputs` for metadata
- Procfile: Simple process management format
- process-compose: Service orchestration tool (inspiration, not dependency)
