# Database Connection & Transaction States

Shows database lifecycle, PRAGMA configuration, and FTS5 trigger synchronization.

## Database Open Sequence

```mermaid
sequenceDiagram
    participant App
    participant DB as Database
    participant SQLite

    App->>DB: Database::open(path, &PragmaConfig)
    DB->>SQLite: Connection::open(path)

    Note over DB,SQLite: Apply PRAGMAs in order
    DB->>SQLite: PRAGMA journal_mode = WAL
    DB->>SQLite: PRAGMA synchronous = NORMAL
    DB->>SQLite: PRAGMA cache_size = -32000 (32MB)
    DB->>SQLite: PRAGMA temp_store = MEMORY
    DB->>SQLite: PRAGMA mmap_size = (platform-aware)
    DB->>SQLite: PRAGMA page_size = 4096

    Note over DB,SQLite: Security PRAGMAs
    DB->>SQLite: PRAGMA foreign_keys = ON
    DB->>SQLite: PRAGMA trusted_schema = OFF
    DB->>SQLite: PRAGMA application_id = 0xA17E6D42

    DB->>SQLite: busy_timeout(5000ms)

    DB-->>App: Ok(Database { conn })
```

## Read-Only Mode (Doctor)

```mermaid
flowchart TD
    A[Database::open_readonly] --> B[Connection::open_with_flags]
    B --> C["SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX"]
    C --> D[Skip PRAGMA writes]
    D --> E[Return Database]

    E --> F{Operations allowed?}
    F -->|SELECT, query_row| G[OK]
    F -->|INSERT, UPDATE, DELETE| H[Error: read-only]
```

## Schema Initialization (Idempotent)

```mermaid
flowchart TD
    A[init_schema] --> B["CREATE TABLE IF NOT EXISTS files"]
    B --> C["CREATE VIRTUAL TABLE IF NOT EXISTS files_fts<br/>USING fts5(path, content, content='files',<br/>content_rowid='id', tokenize='porter unicode61',<br/>columnsize=0)"]

    C --> D["CREATE TRIGGER IF NOT EXISTS files_ai<br/>AFTER INSERT"]
    D --> E["CREATE TRIGGER IF NOT EXISTS files_au<br/>AFTER UPDATE"]
    E --> F["CREATE TRIGGER IF NOT EXISTS files_ad<br/>AFTER DELETE"]

    F --> G["CREATE INDEX IF NOT EXISTS idx_files_mtime"]
    G --> H["CREATE INDEX IF NOT EXISTS idx_files_path"]
    H --> I["CREATE INDEX IF NOT EXISTS idx_files_hash"]

    I --> J[Schema Ready]

    style B fill:#e1f5fe
    style C fill:#fff3e0
    style D fill:#f3e5f5
    style E fill:#f3e5f5
    style F fill:#f3e5f5
    style G fill:#e8f5e9
    style H fill:#e8f5e9
    style I fill:#e8f5e9
```

## FTS5 Trigger Auto-Sync

```mermaid
sequenceDiagram
    participant App
    participant Files as files table
    participant FTS as files_fts

    Note over App,FTS: INSERT Operation
    App->>Files: INSERT INTO files (path, content, ...)
    Files->>FTS: TRIGGER files_ai
    FTS->>FTS: INSERT INTO files_fts(rowid, path, content)<br/>VALUES (new.id, new.path, new.content)

    Note over App,FTS: UPDATE Operation
    App->>Files: UPDATE files SET content = ... WHERE path = ?
    Files->>FTS: TRIGGER files_au
    FTS->>FTS: INSERT INTO files_fts(files_fts, rowid, path, content)<br/>VALUES('delete', old.id, old.path, old.content)
    FTS->>FTS: INSERT INTO files_fts(rowid, path, content)<br/>VALUES (new.id, new.path, new.content)

    Note over App,FTS: DELETE Operation
    App->>Files: DELETE FROM files WHERE path = ?
    Files->>FTS: TRIGGER files_ad
    FTS->>FTS: INSERT INTO files_fts(files_fts, rowid, path, content)<br/>VALUES('delete', old.id, old.path, old.content)
```

## Upsert with Lazy Invalidation

```mermaid
flowchart TD
    A[upsert_file path, content, mtime, size] --> B[hash = wyhash content]
    B --> C[now = Utc::now.timestamp]
    C --> D["INSERT INTO files<br/>(path, content_hash, mtime, size, indexed_at, content)<br/>VALUES (?, ?, ?, ?, ?, ?)"]

    D --> E["ON CONFLICT(path) DO UPDATE SET<br/>content_hash, mtime, size, indexed_at, content"]
    E --> F{"WHERE excluded.content_hash !=<br/>(SELECT content_hash FROM files WHERE path = excluded.path)"}

    F -->|hash differs| G[UPDATE executed]
    G --> H[Triggers fire → FTS5 rebuilt]

    F -->|hash same| I[No update - skip]
    I --> J[No triggers - FTS5 unchanged]

    Note over I,J: Lazy invalidation:<br/>Same content = no work
```

## Schema Validation

```mermaid
flowchart TD
    A[check_schema] --> B[Query sqlite_master]

    B --> C{files table?}
    B --> D{files_fts vtable?}
    B --> E{files_ai trigger?}
    B --> F{files_au trigger?}
    B --> G{files_ad trigger?}
    B --> H{idx_files_mtime?}
    B --> I{idx_files_path?}
    B --> J{idx_files_hash?}

    C --> K[SchemaCheck]
    D --> K
    E --> K
    F --> K
    G --> K
    H --> K
    I --> K
    J --> K

    K --> L{is_complete?}
    L -->|all 8 true| M[Schema valid]
    L -->|any false| N[missing_objects list]
```

## PRAGMA Configuration Reference

| PRAGMA | Default | Platform Notes |
|--------|---------|----------------|
| `journal_mode` | WAL | Enables concurrent readers |
| `synchronous` | NORMAL | Balance durability/performance |
| `cache_size` | -32000 (32MB) | Negative = KB |
| `temp_store` | MEMORY | Temp tables in RAM |
| `mmap_size` | 0 (macOS) / 256MB (Linux) | macOS: unreliable on APFS |
| `page_size` | 4096 | Standard |
| `foreign_keys` | ON | Referential integrity |
| `trusted_schema` | OFF | Security: prevent SQL injection via schema |
| `application_id` | 0xA17E6D42 | Database signature |
| `busy_timeout` | 5000ms | Concurrent access retry |
