# Doctor Diagnostic Check Pipeline

Shows the 10-check sequential diagnostic pipeline.

## Doctor Execution Flow

```mermaid
stateDiagram-v2
    [*] --> New: Doctor::new(project_dir, verbose)

    New --> Run: doctor.run()

    state Run {
        [*] --> Check1: database_exists
        Check1 --> Check2: database_readable
        Check2 --> Check3: application_id
        Check3 --> Check4: schema_complete
        Check4 --> Check5: fts_integrity
        Check5 --> Check6: journal_mode
        Check6 --> Check7: file_count
        Check7 --> Check8: gitignore
        Check8 --> Check9: binary_available
        Check9 --> Check10: orphan_wal_files
        Check10 --> [*]
    }

    Run --> CompileSummary
    CompileSummary --> Output: DoctorSummary
    Output --> [*]: ExitCode
```

## Check Pipeline with Severity

```mermaid
flowchart TD
    subgraph "Critical Checks - Fail = Error Exit"
        A1[1. database_exists] --> A2[2. database_readable]
        A2 --> A3[3. application_id]
        A3 --> A4[4. schema_complete]
        A4 --> A5[5. fts_integrity]
    end

    subgraph "Info Checks - Fail = Warning"
        A5 --> B1[6. journal_mode]
        B1 --> B2[7. file_count]
        B2 --> B3[8. gitignore]
        B3 --> B4[9. binary_available]
        B4 --> B5[10. orphan_wal_files]
    end

    B5 --> C[Compile Summary]

    style A1 fill:#ffcdd2
    style A2 fill:#ffcdd2
    style A3 fill:#ffcdd2
    style A4 fill:#ffcdd2
    style A5 fill:#ffcdd2
    style B1 fill:#fff9c4
    style B2 fill:#fff9c4
    style B3 fill:#fff9c4
    style B4 fill:#fff9c4
    style B5 fill:#fff9c4
```

## Individual Check Details

### Check 1: database_exists
```mermaid
flowchart TD
    A[fs::metadata db_path] --> B{exists?}
    B -->|yes| C["Pass: Database found (size bytes)"]
    B -->|no| D["Error: Database missing<br/>Remediation: Run ffts-grep init"]
```

### Check 2: database_readable
```mermaid
flowchart TD
    A[Database::open_readonly] --> B{success?}
    B -->|yes| C[Pass: Database readable]
    B -->|no| D["Error: Cannot open database<br/>Remediation: Check permissions"]
```

### Check 3: application_id
```mermaid
flowchart TD
    A[db.get_application_id] --> B{== 0xA17E6D42?}
    B -->|yes| C[Pass: Correct application ID]
    B -->|no| D["Error: Wrong application ID<br/>Remediation: This is a different app's DB"]
```

### Check 4: schema_complete
```mermaid
flowchart TD
    A[db.check_schema] --> B{is_complete?}
    B -->|yes| C["Pass: All 8 objects present<br/>(2 tables, 3 triggers, 3 indexes)"]
    B -->|no| D["Error: Missing schema objects<br/>Remediation: Run ffts-grep init --force"]
    D --> E[List missing objects]
```

### Check 5: fts_integrity
```mermaid
flowchart TD
    A["INSERT INTO files_fts(files_fts)<br/>VALUES('integrity-check')"] --> B{success?}
    B -->|yes| C[Pass: FTS5 index valid]
    B -->|no| D["Error: FTS5 corruption detected<br/>Remediation: Run ffts-grep init --force"]
```

### Check 6: journal_mode
```mermaid
flowchart TD
    A[db.get_journal_mode] --> B{== 'wal'?}
    B -->|yes| C[Pass: WAL mode enabled]
    B -->|no| D["Warning: Not using WAL mode<br/>Remediation: Run ffts-grep init --force"]
```

### Check 7: file_count
```mermaid
flowchart TD
    A[db.get_file_count] --> B{count > 0?}
    B -->|yes| C["Pass: N files indexed"]
    B -->|no| D["Warning: No files indexed<br/>Remediation: Run ffts-grep index"]
```

### Check 8: gitignore
```mermaid
flowchart TD
    A[init::check_gitignore] --> B{all entries present?}
    B -->|yes| C[Pass: .gitignore configured]
    B -->|no| D["Warning: Missing gitignore entries<br/>Remediation: Run ffts-grep init --gitignore-only"]
```

### Check 9: binary_available
```mermaid
flowchart TD
    A[std::env::current_exe] --> B{success?}
    B -->|yes| C["Pass: Binary at path"]
    B -->|no| D["Warning: Cannot determine binary path"]
```

### Check 10: orphan_wal_files
```mermaid
flowchart TD
    A["Check .ffts-index.db-shm<br/>and .ffts-index.db-wal"] --> B{exist without main DB?}
    B -->|no| C[Pass: No orphan files]
    B -->|yes| D["Warning: Orphan WAL files found<br/>Remediation: Delete manually or run init --force"]
```

## Output Format States

```mermaid
flowchart TD
    A[DoctorSummary] --> B{format?}

    B -->|Plain| C{verbose?}
    C -->|yes| D["[N/10] Check name<br/>Details<br/>Status<br/>Remediation"]
    C -->|no| E["Symbol Message<br/>→ Remediation"]

    B -->|Json| F[DoctorOutput struct]
    F --> G["{ version, project_dir,<br/>checks, summary, exit_code }"]

    subgraph "Status Symbols"
        H["✓ Pass"]
        I["ℹ Info"]
        J["⚠ Warning"]
        K["✗ Error"]
    end
```

## Exit Code Calculation

```mermaid
flowchart TD
    A[Summary] --> B{fail_count > 0?}
    B -->|yes| C["Exit 2 - DATAERR<br/>(Database issues)"]
    B -->|no| D{warn_count > 0?}
    D -->|yes| E["Exit 1 - SOFTWARE<br/>(Non-critical issues)"]
    D -->|no| F["Exit 0 - OK"]
```

## Check Summary Table

| # | Check | Severity | Pass Condition |
|---|-------|----------|----------------|
| 1 | database_exists | Error | File exists |
| 2 | database_readable | Error | Can open read-only |
| 3 | application_id | Error | ID == 0xA17E6D42 |
| 4 | schema_complete | Error | All 8 objects exist |
| 5 | fts_integrity | Error | integrity-check passes |
| 6 | journal_mode | Warning | mode == 'wal' |
| 7 | file_count | Warning | count > 0 |
| 8 | gitignore | Warning | All 4 entries present |
| 9 | Binary availability | Warning | current_exe() succeeds |
| 10 | orphan_wal_files | Warning | No orphan -shm/-wal files |
