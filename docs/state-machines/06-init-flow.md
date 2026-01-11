# Project Initialization State Machine

Shows the init command flow including gitignore updates and force mode.

## Init Execution Flow

```mermaid
stateDiagram-v2
    [*] --> UpdateGitignore: run_init()

    state UpdateGitignore {
        [*] --> CallUpdate: init::update_gitignore()
        CallUpdate --> GitignoreResult: Ok
        CallUpdate --> ExitIoErr: Err
    }

    GitignoreResult --> CheckGitignoreOnly

    CheckGitignoreOnly --> OutputAndExit: gitignore_only=true
    CheckGitignoreOnly --> CheckDbExists: gitignore_only=false

    state CheckDbExists {
        [*] --> DbExists: db_path.exists()
        DbExists --> OpenExisting: exists && !force
        DbExists --> DeleteExisting: exists && force
        DbExists --> CreateNew: !exists
    }

    state OpenExisting {
        [*] --> OpenDb: Database::open()
        OpenDb --> GetCount: db.get_file_count()
        GetCount --> OutputResult: report existing count
    }

    state DeleteExisting {
        [*] --> RemoveDb: fs::remove_file(db)
        RemoveDb --> RemoveShm: fs::remove_file(db-shm)
        RemoveShm --> RemoveWal: fs::remove_file(db-wal)
        RemoveWal --> CreateNew
    }

    state CreateNew {
        [*] --> OpenNewDb: Database::open()
        OpenNewDb --> InitSchema: db.init_schema()
        InitSchema --> IndexFiles: Indexer::new() + index_directory()
        IndexFiles --> OutputResult
    }

    OutputResult --> OutputInitResult
    OutputInitResult --> ExitOk

    OutputAndExit --> ExitOk
```

## Gitignore Update State Machine

```mermaid
flowchart TD
    A[update_gitignore] --> B[Read .gitignore content]
    B --> C{File exists?}

    C -->|no| D[content = empty string]
    C -->|yes| E[content = file contents]

    D --> F[Parse patterns]
    E --> F

    F --> G["HashSet of existing patterns<br/>(trim, skip comments)"]

    G --> H{Check required entries}

    subgraph "Required Entries"
        I[.ffts-index.db]
        J[.ffts-index.db-shm]
        K[.ffts-index.db-wal]
        L[.ffts-index.db.tmp]
    end

    H --> M{All present?}
    M -->|yes| N[GitignoreResult::AlreadyComplete]

    M -->|no| O[Build new content]
    O --> P[Preserve existing content]
    P --> Q["Add header comment<br/># ffts-grep index files"]
    Q --> R[Add missing entries]
    R --> S[Write to .gitignore.tmp]
    S --> T[fs::rename tmp -> .gitignore]

    T --> U{File existed before?}
    U -->|yes| V["GitignoreResult::Updated(N)"]
    U -->|no| W["GitignoreResult::Created(N)"]
```

## Force Reinit Cleanup

```mermaid
sequenceDiagram
    participant Init as run_init
    participant FS as Filesystem
    participant DB as Database
    participant IDX as Indexer

    Note over Init,IDX: force=true && db_exists
    Init->>FS: fs::remove_file(.ffts-index.db)
    Init->>FS: fs::remove_file(.ffts-index.db-shm)
    Init->>FS: fs::remove_file(.ffts-index.db-wal)
    Note over FS: All removals ignore errors<br/>(files may not exist)

    Init->>DB: Database::open(db_path)
    Init->>DB: init_schema()

    Init->>IDX: Indexer::new(project_dir, db)
    Init->>IDX: index_directory()

    IDX-->>Init: IndexStats
```

## InitResult Structure

```mermaid
classDiagram
    class InitResult {
        +GitignoreResult gitignore
        +bool database_created
        +usize files_indexed
    }

    class GitignoreResult {
        <<enumeration>>
        AlreadyComplete
        Created(usize)
        Updated(usize)
    }

    InitResult --> GitignoreResult
```

## CLI Flags Impact

| Flag | Gitignore | Database | Indexing |
|------|-----------|----------|----------|
| (none) | Update if needed | Open existing or create | Index if new |
| `--gitignore-only` | Update if needed | Skip | Skip |
| `--force` | Update if needed | Delete + recreate | Full reindex |
| `--force --gitignore-only` | Update if needed | Skip | Skip |

## Output Flow

```mermaid
flowchart TD
    A[InitResult] --> B{quiet?}
    B -->|yes| C[Return silently]
    B -->|no| D[output_init_result to stderr]

    D --> E{gitignore result}
    E -->|AlreadyComplete| F[".gitignore already configured"]
    E -->|Created N| G["Created .gitignore with N entries"]
    E -->|Updated N| H["Updated .gitignore with N entries"]

    D --> I{database_created?}
    I -->|true| J["Database created, N files indexed"]
    I -->|false| K["Database exists, N files indexed"]
```

## Error Exit Points

| Error | Exit Code | Message |
|-------|-----------|---------|
| Gitignore write fail | IOERR (74) | Failed to update .gitignore |
| DB open fail | IOERR (74) | Failed to open/create database |
| Schema init fail | SOFTWARE (70) | Failed to initialize schema |
| Index fail | SOFTWARE (70) | Indexing failed during initialization |
| Output fail | SOFTWARE (70) | Failed to output init results |
