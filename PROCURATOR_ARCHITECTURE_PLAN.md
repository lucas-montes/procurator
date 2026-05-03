# Procurator Deep Architecture Plan
## GitHub-Alternative + Gerrit Patchsets + Quality Intelligence + AI-Native

---

## 1. Understanding What You Already Have

Your codebase already has a **solid foundation**:

```
┌─────────────────────────────────────────────────────────┐
│                    PROCURATOR                           │
├─────────────────────────────────────────────────────────┤
│  repohub: GitHub UI + Gerrit Review (both working)    │
│  ci_service: Build queue + Nix flake check (partial)   │
│  worker: Cloud Hypervisor VM management (mostly done)  │
│  control_plane: State management (scaffolded)          │
│  autonix: Auto-generates Nix flakes from repos         │
│  repo_outils: Nix flake parsing, git operations        │
└─────────────────────────────────────────────────────────┘
```

**Key insight**: You have *parallel* GitHub and Gerrit interfaces. The Gerrit model with **PatchSets** is powerful - it allows iterative improvement of a change before merge. This is where quality metrics should live.

---

## 2. The Core Idea: Project as Quality Context

### Current Model
```
User → Project → Repository (many)
                → Repository (many)
```

### Enhanced Model
```
User → Project → ProjectMetadata (NEW - quality context for ALL repos)
             ├── cache_build_url
             ├── infra_config (from your existing Infrastructure struct)
             ├── quality_metrics (NEW)
             ├── fitness_functions (NEW)
             ├── diagrams (NEW)
             ├── testing_strategy (NEW)
             └── ai_agent_configs (NEW)
                │
                ├── Repository → Change → PatchSet (enriched with metrics)
                ├── Repository → Change → PatchSet (enriched with metrics)
                └── Repository → Change → PatchSet (enriched with metrics)
```

**Critical insight**: ProjectMetadata is the **quality contract** that applies to all repos in the project. When you change a fitness function threshold at project level, it affects the submit readiness of ALL changes across ALL repos.

---

## 3. Project Metadata Synchronization

### The Problem
You have multiple repos in a project. Settings like "require 80% test coverage" should be defined once and apply everywhere. When you update the cache build URL, all repos should see it.

### Proposed Solution: Three-Layer Sync

```
┌──────────────────────────────────────────────────────┐
│ Layer 1: PRIMARY STORE (SQLite in repohub)         │
│ - ProjectMetadata table with version field          │
│ - Canonical source of truth                        │
└────────────────┬───────────────────────────────────┘
                 │
        ┌────────┴────────┐
        │                 │
        ▼                 ▼
┌──────────────┐  ┌──────────────────┐
│ Layer 2:     │  │ Layer 3:        │
│ Event Bus    │  │ Git-Backed      │
│ (NATS/Redis) │  │ (.repohub/)     │
└──────┬───────┘  └──────┬──────────┘
       │                  │
       │ Subscribe        │ Read at
       │                  │ repo clone
       ▼                  ▼
┌──────────────┐  ┌──────────────────┐
│ Repos react  │  │ .repohub/        │
│ in real-time │  │ metadata.toml    │
│ to changes   │  │ (version control)│
└──────────────┘  └──────────────────┘
```

### Why Three Layers?

| Layer | Purpose | When Used |
|-------|---------|-----------|
| **Primary Store** | CRUD, API, UI | User updates settings via web UI |
| **Event Bus** | Real-time sync | Repo A changes → Repo B sees update in seconds |
| **Git-Backed** | Offline, versioned | Developer clones repo, reads `.repohub/metadata.toml` |

### Implementation Detail

**Primary Store** - Extend your existing SQLite:
```rust
// New table in repohub
CREATE TABLE project_metadata (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    version INTEGER NOT NULL DEFAULT 1,
    
    -- Cache & Build
    cache_build_url TEXT,
    binary_cache_url TEXT,
    
    -- Serialized JSON for complex types
    infra_config_json TEXT,      -- Infrastructure struct
    quality_metrics_json TEXT,    -- QualityMetricsSnapshot
    fitness_functions_json TEXT,  -- Vec<FitnessFunction>
    diagrams_json TEXT,           -- Vec<Diagram>
    testing_strategy_json TEXT,   -- TestingStrategy
    ai_agent_configs_json TEXT,   -- Vec<AIAgentConfig>
    
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, version)
);
```

**Event Bus** - Simple NATS publish (can start with in-process channel):
```rust
pub enum ProjectEvent {
    MetadataUpdated { project_id: i64, version: i64 },
    FitnessFunctionChanged { project_id: i64, function_id: String },
    DiagramGenerated { project_id: i64, diagram_id: String },
}
```

**Git-Backed** - Export to repos:
```toml
# .repohub/metadata.toml
[project]
cache_build_url = "https://cache.example.com"
binary_cache_url = "https://binary-cache.example.com"

[testing_strategy]
approach = "test-driven"
frameworks = ["pytest", "jest"]

[[fitness_functions]]
name = "test-coverage"
type = "TestCoverage"
threshold = 0.80
weight = 1.0
```

---

## 4. Quality Metrics + Fitness Functions

### The Gerrit PatchSet is the Perfect Place for Quality

Your current `PatchSet` is simple. Let's enrich it:

```rust
pub struct PatchSet {
    // Existing fields
    pub change_id: i64,
    pub number: i32,
    pub revision: String,
    pub kind: PatchSetKind,
    pub uploader_user_id: i64,
    
    // NEW: Quality enrichment (populated by CI service)
    pub build_url: Option<String>,           // Link to CI build
    pub cache_build_url: Option<String>,     // From project settings
    pub test_coverage: Option<f64>,          // 0.0 to 1.0
    pub quality_score: Option<f64>,         // Weighted fitness score
    pub fitness_results: Vec<FitnessResult>, // Per-function results
    pub metrics: HashMap<String, MetricValue>, // Extensible metrics
    pub diagrams_generated: Vec<String>,    // Diagram IDs
}
```

### How Fitness Functions Work

Borrowing from *Building Evolutionary Architectures*:

```rust
pub struct FitnessFunction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub function_type: FitnessFunctionType,
    pub threshold: f64,      // Minimum passing score
    pub weight: f64,         // Importance in aggregate (0.0 - 1.0)
    pub enabled: bool,
    pub scope: FitnessScope, // Per-patchset or per-project?
}

pub enum FitnessFunctionType {
    // Code quality metrics
    TestCoverage { min_percentage: f64 },
    CyclomaticComplexity { max_per_function: f64 },
    DuplicateCode { max_percentage: f64 },
    TechDebtRatio { max_ratio: f64 },
    
    // Architecture guards
    DependencyCheck { 
        forbidden: Vec<String>,  // e.g., ["unwrap", "panic"]
    },
    LayeringCheck { 
        allowed: HashMap<String, Vec<String>>, // layer → allowed deps
    },
    
    // Performance
    BuildTime { max_seconds: f64 },
    TestExecutionTime { max_seconds: f64 },
    
    // Custom - run a script, check output
    CustomScript { 
        script: String,      // e.g., "cargo clippy --message-format=json"
        language: String,    // "bash", "python", etc.
        parse_as: String,    // "json", "text", "numeric"
    },
}

pub struct FitnessResult {
    pub function_id: String,
    pub score: f64,          // 0.0 to 1.0 (normalized)
    pub threshold: f64,
    pub passed: bool,
    pub details: Option<String>,
    pub execution_time_ms: i64,
}
```

### How CI Service Fits In

Your `ci_service` currently runs `nix flake check`. Extend it:

```
PatchSet Created
       │
       ▼
ci_service receives webhook/build request
       │
       ├── Run nix flake check (existing)
       ├── Run fitness functions (NEW)
       │   ├── Test coverage (lcov, etc.)
       │   ├── Static analysis (clippy, etc.)
       │   └── Custom scripts
       ├── Collect metrics (NEW)
       ├── Generate diagrams (NEW - from code analysis)
       └── Emit results to MetricsCollector
              │
              ▼
       Update PatchSet in repohub DB
              │
              ▼
       Check SubmitReadiness (includes quality gate)
```

### SubmitReadiness Enhancement

Your current `SubmitReadiness` checks labels (Code-Review, Verified). Add quality gate:

```rust
pub struct SubmitReadiness {
    pub ready: bool,
    pub checks: BTreeMap<String, bool>,
    pub quality_gate: QualityGateResult,  // NEW
}

pub struct QualityGateResult {
    pub passed: bool,
    pub overall_score: f64,
    pub required_coverage: Option<f64>,
    pub actual_coverage: Option<f64>,
    pub fitness_functions_passed: bool,
    pub failing_functions: Vec<String>,
    pub flaky_tests_detected: bool,
}
```

Now when someone tries to submit a change, they see:
```
Submit Readiness:
  ✗ Code-Review >= +2 (currently: +1)
  ✗ Verified >= +1 (not yet)
  ✗ Test Coverage >= 80% (currently: 65%)
  ✗ Cyclomatic Complexity <= 10 (failed: 3 functions exceed)
  
  Overall: NOT READY
```

---

## 5. Diagram Generation

### What Diagrams Do You Need?

Based on your notes mentioning SvelteFlow and infrastructure understanding:

| Diagram Type | Source | Generation Method |
|--------------|--------|-------------------|
| **Infrastructure topology** | Nix `infrastructure` attr | Parse `Infrastructure` struct → Mermaid/SvelteFlow |
| **Service dependencies** | `ProjectConfiguration` deps | Directed graph from `DependencyEdge` |
| **Code architecture** | Static analysis of repo | Tree-sitter → module graph |
| **Data flow** | API definitions, service connections | From service config |
| **Deployment pipeline** | CD config in `Infrastructure` | Mermaid graph |
| **VM cluster** | Control plane state | Runtime: query control plane API |

### Diagram Generator Architecture

```
┌─────────────────────────────────────────────────┐
│           DiagramGenerator trait                 │
│                                                 │
│  async fn generate_infra_diagram() → Diagram    │
│  async fn generate_service_deps() → Diagram     │
│  async fn generate_code_arch() → Diagram        │
│  async fn generate_data_flow() → Diagram        │
└────────────┬────────────────────────────────────┘
             │
    ┌────────┼────────┐
    │        │        │
    ▼        ▼        ▼
┌────────┐ ┌────────┐ ┌────────┐
│Mermaid │ │Svelte  │ │ SVG    │
│Output  │ │Flow    │ │Render  │
│        │ │JSON    │ │        │
└────────┘ └────────┘ └────────┘
```

### Implementation Approach

Your `repo_outils` already parses `Infrastructure` from flake. Build on that:

```rust
// Generate Mermaid from Infrastructure
pub fn infra_to_mermaid(infra: &Infrastructure) -> String {
    let mut mermaid = String::from("graph TD\n");
    
    // Add machines
    for (name, machine) in &infra.machines {
        mermaid.push_str(&format!("  {name}[Machine: {name}<br/>CPU: {} Memory: {}]<br/>Roles: {:?}]\n", 
            machine.cpu, machine.memory, machine.roles));
    }
    
    // Add services
    for (name, service) in &infra.services {
        mermaid.push_str(&format!("  {name}[Service: {name}]]\n"));
    }
    
    // Add connections from ProjectConfiguration deps
    // ...
    
    mermaid
}
```

Store diagrams in DB:
```rust
pub struct Diagram {
    pub id: String,
    pub project_id: i64,
    pub diagram_type: DiagramType,
    pub title: String,
    pub content: String,      // Mermaid syntax or SvelteFlow JSON
    pub format: DiagramFormat,
    pub generated: bool,      // auto-generated vs manual
    pub source: String,       // what generated it
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

---

## 6. AI Agent Integration

### Your Vision (from notes.md)
> "An agent or some AI bullshit that reads everything and writes and keeps documentation up to date"

### Agent Architecture

```
┌──────────────────────────────────────────────────────┐
│                  AI Agent Gateway                    │
│                                                      │
│  Event Bus Subscription                             │
│  ├── ChangeCreated → Document the change            │
│  ├── PatchSetUploaded → Analyze code quality       │
│  ├── ChangeMerged → Update architecture docs       │
│  └── DiagramGenerated → Add diagram explanations   │
│                                                      │
│  Agent Registry                                     │
│  ├── DocumentationAgent                             │
│  ├── CodeAnalysisAgent                              │
│  ├── DiagramGeneratorAgent                          │
│  └── Custom agents (configurable)                   │
└──────────────────┬───────────────────────────────────┘
                   │
        ┌──────────┼──────────┐
        │          │          │
        ▼          ▼          ▼
   ┌────────┐ ┌────────┐ ┌────────┐
   │ Local  │ │ Remote │ │ Cloud  │
   │ Agent  │ │ Agent  │ │ AI API │
   │ (LLM)  │ │ (HTTP) │ │ (API)  │
   └────────┘ └────────┘ └────────┘
```

### Agent Configuration Model

```rust
pub struct AIAgentConfig {
    pub id: String,
    pub name: String,
    pub agent_type: AIAgentType,
    pub endpoint: Option<String>,  // None = local, Some(url) = remote
    pub capabilities: Vec<AgentCapability>,
    pub triggers: Vec<AgentTrigger>,
    pub config: serde_json::Value,  // Agent-specific (API keys, etc.)
    pub enabled: bool,
}

pub enum AIAgentType {
    Documentation,
    CodeAnalysis,
    DiagramGeneration,
    FitnessEvaluation,
    TestGeneration,
    Custom { handler: String },
}

pub enum AgentTrigger {
    ChangeCreated,
    PatchSetUploaded,
    ChangeMerged,
    DiagramGenerated,
    ManualTrigger,
    Scheduled { cron: String },
}
```

### Agent Protocol (Simple REST)

```rust
// Event sent to agents
pub struct AgentEvent {
    pub event_id: String,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub project_id: i64,
    pub repo_id: Option<i64>,
    pub change_id: Option<i64>,
    pub patch_set_id: Option<i64>,
    pub payload: serde_json::Value,
}

// Agent response
pub struct AgentResult {
    pub success: bool,
    pub actions_taken: Vec<AgentAction>,
    pub suggestions: Vec<String>,
    pub diagrams_generated: Vec<String>,
    pub docs_updated: Vec<String>,
}
```

### Example: Documentation Agent

When a change is merged:
1. Agent receives `ChangeMerged` event
2. Agent reads the diff (via git)
3. Agent generates/updates documentation:
   - README updates
   - Architecture docs
   - API documentation
   - Inline code comments
4. Agent creates a new PatchSet with documentation changes
5. Agent votes on the change with documentation quality

---

## 7. Complete Data Flow

```
┌─────────────────────────────────────────────────────────────┐
│                    DEVELOPER WORKFLOW                        │
│                                                             │
│  1. Developer pushes code                                   │
│     └──→ Git Server (bare repo)                             │
│         └──→ Post-receive hook                              │
│             └──→ CI Service (build + fitness functions)     │
│                 ├── Run tests                               │
│                 ├── Check coverage                          │
│                 ├── Run static analysis                     │
│                 ├── Generate diagrams                       │
│                 └── Emit metrics to MetricsCollector        │
│                     └── Update PatchSet in repohub DB       │
│                                                             │
│  2. Developer creates Change (Gerrit)                      │
│     └──→ Change created with PatchSet #1                    │
│         └──→ Event: ChangeCreated                          │
│             ├──→ AI Agent: analyze code, suggest improvements│
│             └──→ Repohub: show change UI                   │
│                                                             │
│  3. Developer uploads new PatchSet                          │
│     └──→ PatchSet #2 created                               │
│         └──→ CI runs, metrics collected                    │
│             └──→ PatchSet enriched with quality data        │
│                 └──→ SubmitReadiness updated               │
│                     └──→ UI shows quality gate status      │
│                                                             │
│  4. Reviewer votes                                         │
│     └──→ Code-Review +2, Verified +1                       │
│         └──→ SubmitReadiness: all checks pass              │
│             └──→ Change can be submitted                   │
│                                                             │
│  5. Change submitted (merged)                               │
│     └──→ Event: ChangeMerged                              │
│         └──→ AI Agent: update documentation               │
│             └──→ New PatchSet with doc updates             │
│                 └──→ Diagram generation updated            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

---

## 8. Implementation Phases (Detailed)

### Phase 1: Project Metadata Foundation (Week 1-2)

**Goal**: Store and sync project-level settings

- [ ] Create `ProjectMetadata` domain model
- [ ] Add SQLite migration for `project_metadata` table
- [ ] Create `ProjectMetadataPort` trait (follow your ports pattern)
- [ ] Implement `SqliteProjectMetadataRepository`
- [ ] Extend `PatchSet` with quality fields
- [ ] Add API endpoints:
  - `GET /{user}/{project}/settings/metadata`
  - `POST /{user}/{project}/settings/metadata`
  - `GET /api/projects/{id}/metadata`
- [ ] Create Askama templates for metadata UI
- [ ] Implement simple event bus (in-process `tokio::sync::broadcast`)
- [ ] Export metadata to `.repohub/metadata.toml`

### Phase 2: Quality Metrics Collection (Week 2-3)

**Goal**: CI service collects and stores metrics

- [ ] Extend `ci_service` to run fitness functions
- [ ] Create `FitnessFunction` domain model
- [ ] Create `MetricsCollector` service (new crate or in repohub)
- [ ] Parse test coverage (lcov, cobertura formats)
- [ ] Parse static analysis output (clippy JSON, etc.)
- [ ] Store `FitnessResult` per PatchSet
- [ ] Add `QualityGateResult` to `SubmitReadiness`
- [ ] Display quality dashboard on Change detail page

### Phase 3: Diagram Generation (Week 3-4)

**Goal**: Auto-generate infrastructure and code diagrams

- [ ] Create `DiagramGenerator` trait
- [ ] Implement `MermaidGenerator`:
  - From `Infrastructure` struct (your existing one in repo_outils)
  - From `ProjectConfiguration` dependencies
- [ ] Implement `CodeArchGenerator`:
  - Use tree-sitter to parse code
  - Generate module dependency graph
- [ ] Store diagrams in DB
- [ ] Display diagrams in UI (embed Mermaid.js or SvelteFlow)
- [ ] Add "Generate Diagrams" button to repo/project pages

### Phase 4: AI Agent Gateway (Week 4-5)

**Goal**: Agents can react to events and take actions

- [ ] Create `AIAgentConfig` domain model
- [ ] Build agent registry (store configs in DB)
- [ ] Implement event dispatcher (subscribe agents to events)
- [ ] Create simple local DocumentationAgent:
  - Uses local LLM (ollama) or API
  - Reads code, generates/updates docs
- [ ] Create simple CodeAnalysisAgent:
  - Suggests improvements based on quality metrics
- [ ] Add agent management UI
- [ ] Wire up events: ChangeCreated, PatchSetUploaded, ChangeMerged

### Phase 5: Integration & Polish (Week 5-6)

**Goal**: Everything works together

- [ ] Wire quality gate into submit readiness check
- [ ] Project dashboard shows all repos' quality metrics
- [ ] Diagrams appear in change review (infra context)
- [ ] AI agents automatically document merged changes
- [ ] Add tests for new functionality
- [ ] Update documentation

---

## 9. Key Technical Decisions

| Decision | Choice | Reasoning |
|----------|--------|-----------|
| **Event Bus** | Start with `tokio::sync::broadcast`, upgrade to NATS later | Simple, works in-process, can swap later |
| **Diagram Format** | Mermaid (primary) | Text-based, git-friendly, widely supported, renders in GitHub/GitLab |
| **Metrics Storage** | JSON in SQLite (like your existing pattern) | Simple, queryable with `json_extract` |
| **AI Agent Protocol** | REST/JSON (agents implement simple HTTP endpoint) | Flexible, agents can be any language |
| **Fitness Functions** | Configurable in project settings | Project-specific quality gates |
| **Sync Mechanism** | Event + Git hybrid | Real-time + version-controlled |
| **Quality Gate** | Part of `SubmitReadiness` | Fits existing Gerrit model |

---

## 10. Code Location Reference

Based on your existing patterns:

```
repohub/src/
├── domain/
│   ├── github.rs          (existing: User, Project, Repository)
│   ├── review.rs          (existing: Change, PatchSet, ReviewPolicy)
│   ├── configuration.rs   (existing: ProjectConfiguration)
│   ├── project_metadata.rs (NEW: ProjectMetadata, Diagram, etc.)
│   └── fitness.rs         (NEW: FitnessFunction, FitnessResult)
├── application/
│   ├── github/ports.rs    (existing: GithubPort)
│   ├── gerrit/ports.rs    (existing: Change*Port, PolicyPort)
│   └── metadata/ports.rs  (NEW: ProjectMetadataPort)
├── adapters/
│   ├── github/web.rs      (existing: GitHub UI routes)
│   ├── gerrit/web.rs      (existing: Gerrit UI routes)
│   └── metadata/web.rs    (NEW: metadata UI routes)
├── services/
│   ├── repository_service.rs (existing)
│   └── metrics_collector.rs  (NEW)
└── templates/
    ├── *.html (existing)
    └── project_metadata*.html (NEW)
```

---

## 11. Existing Code References

### Key Files to Extend

| File | What to Add |
|------|-------------|
| `repohub/src/domain/review.rs` | Extend `PatchSet` struct with quality fields |
| `repohub/src/domain/configuration.rs` | Keep as-is, reference from ProjectMetadata |
| `repohub/src/adapters/shared/database.rs` | Add `project_metadata` table creation |
| `repohub/src/adapters/github/web.rs` | Add metadata settings routes |
| `repohub/src/adapters/gerrit/web.rs` | Show quality metrics in change detail |
| `ci_service/src/` | Add fitness function execution |
| `repo_outils/src/nix/flake.rs` | Leverage existing `Infrastructure` struct |

### Existing Data Models to Reuse

- `Infrastructure` from `repo_outils/src/nix/flake.rs` - for infra diagrams
- `ProjectConfiguration` from `repohub/src/domain/configuration.rs` - for service deps
- `Change`, `PatchSet` from `repohub/src/domain/review.rs` - enrich with quality
- `ReviewPolicy`, `SubmitReadiness` - extend with quality gate

---

*Last Updated: May 2026*
*Based on codebase exploration and notes.md vision*
