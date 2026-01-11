# State Machine Diagrams

This document contains state machine diagrams for the ffts-grep components.

## 1. Main Application Flow

```mermaid
stateDiagram-v2
    [*] --> ParseArgs: main()

    state ParseArgs {
        [*] --> ReadArgs
        ReadArgs --> CheckFlag: --help
        ReadArgs --> CheckFlag: --version
        ReadArgs --> CheckFlag: --index
        ReadArgs --> CheckFlag: --reindex
        ReadArgs --> CheckFlag: --benchmark
        ReadArgs --> CheckFlag: --json
        ReadArgs --> CheckFlag: --paths
        ReadFlag --> ExtractValue: --project-dir=*
        ReadArgs --> ExtractQuery: positional arg
        CheckFlag --> [*]
    }

    ParseArgs --> ResolveProjectDir
    ResolveProjectDir --> BuildDbPath

    state BuildDbPath {
        [*] --> CheckAbsolute
        CheckAbsolute --> AbsolutePath
        CheckAbsolute --> RelativePath: not absolute
        RelativePath --> GetCwd
        GetCwd --> JoinPath
        AbsolutePath --> [*]
        JoinPath --> [*]
    }

    BuildDbPath --> OpenDatabase

    OpenDatabase --> Decision

    state Decision {
        [*] --> CheckReindex
        CheckReindex --> DeleteDb: opts.reindex == true
        CheckReindex --> OpenExisting: opts.reindex == false
        DeleteDb --> InitNewDb
        InitNewDb --> RunIndexing
        OpenExisting --> CheckIndex
        CheckIndex --> RunIndexing: opts.index == true
        CheckIndex --> CheckBenchmark
        CheckBenchmark --> RunBenchmark: opts.benchmark == true
        CheckBenchmark --> PrepareQuery: opts.benchmark == false
    }

    state RunIndexing {
        [*] --> CreateIndexer
        CreateIndexer --> WalkDirectory
        WalkDirectory --> ProcessFile
        ProcessFile --> CheckBatchSize
        CheckBatchSize --> CommitTransaction: batch_size >= limit
        CheckBatchSize --> ProcessFile: more files
        CommitTransaction --> ProcessFile
        ProcessFile --> CheckMoreFiles
        CheckMoreFiles --> CloseTransaction: no more files
        CloseTransaction --> [*]
    }

    state PrepareQuery {
        [*] --> CheckQueryArg
        CheckQueryArg --> UseArg: query provided
        CheckQueryArg --> ReadStdin: no query arg
        ReadStdin --> ParseJsonOrPlain
        ParseJsonOrPlain --> CreateSearcher
        UseArg --> CreateSearcher
    }

    state CreateSearcher {
        [*] --> BuildConfig
        BuildConfig --> InitSearcher
        InitSearcher --> ExecuteSearch
    }

    state ExecuteSearch {
        [*] --> CheckEmptyQuery
        CheckEmptyQuery --> DbGetAllFiles: query == ""
        CheckEmptyQuery --> DbSearch: query != ""
        DbGetAllFiles --> FormatResults
        DbSearch --> FormatResults
    }

    state FormatResults {
        [*] --> CheckJsonFlag
        CheckJsonFlag --> FormatJson: opts.json == true
        CheckJsonFlag --> FormatPlain: opts.json == false
        FormatJson --> OutputResults
        FormatPlain --> OutputResults
    }

    OutputResults --> [*]
```

## 2. Indexer State Machine

```mermaid
stateDiagram-v2
    [*] --> Idle

    state Idle {
        [*] --> indexDirectory
        indexDirectory --> LoadIgnoreRules
    }

    state LoadIgnoreRules {
        [*] --> CheckGitignoreExists
        CheckGitignoreExists --> ParseRules: .gitignore found
        CheckGitignoreExists --> DefaultRules: no .gitignore
        DefaultRules --> [*]
        ParseRules --> [*]
    }

    state WalkDirectory {
        [*] --> OpenDir
        OpenDir --> CreateWalker
        CreateWalker --> NextEntry
        NextEntry --> [*]: no more entries
        NextEntry --> ValidateEntry

        state ValidateEntry {
            [*] --> CheckSymlink
            CheckSymlink --> Skip: follow_symlinks == false
            CheckSymlink --> CheckIgnored
            CheckIgnored --> Skip: should ignore
            CheckIgnored --> IsDirectory
            IsDirectory --> Skip: entry is dir
            IsDirectory --> BuildFullPath
        }

        BuildFullPath --> ValidateAbsolutePath
        ValidateAbsolutePath --> Skip: not absolute
        ValidateAbsolutePath --> ProcessFile
    }

    state ProcessFile {
        [*] --> OpenFile
        OpenFile --> ResolveSymlink
        ResolveSymlink --> CheckWithinRoot
        CheckWithinRoot --> Skip: outside root
        CheckWithinRoot --> GetFileStat
        GetFileStat --> CheckSize
        CheckSize --> Skip: too large
        CheckSize --> ReadContent
        ReadContent --> ValidateUtf8
        ValidateUtf8 --> Skip: invalid encoding
        ValidateUtf8 --> BeginTransaction: not in tx
        ValidateUtf8 --> UpsertFile: already in tx
        BeginTransaction --> UpsertFile
        UpsertFile --> IncrementCount
        IncrementCount --> CheckBatchLimit
    }

    state CheckBatchLimit {
        [*] --> CheckSize
        CheckSize --> CommitTransaction: batch full
        CheckSize --> ContinueWalking: batch not full
        CommitTransaction --> ContinueWalking
    }

    state ContinueWalking {
        [*] --> NextEntry
    }

    Skip --> NextEntry
```

## 3. Database Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Init

    state Init {
        [*] --> Allocate
        Allocate --> CreateConnection
        CreateConnection --> OpenDatabase
        OpenDatabase --> CheckResult
        CheckResult --> Error: failed
        CheckResult --> ApplyPragmas
    }

    state ApplyPragmas {
        [* --> SetJournalMode
        SetJournalMode --> SetSynchronous
        SetSynchronous --> SetCacheSize
        SetCacheSize --> SetTempStore
        SetTempStore --> SetMmapSize
        SetMmapSize --> SetPageSize
    }

    Init --> InitSchema

    state InitSchema {
        [*] --> CreateFilesTable
        CreateFilesTable --> CreateFts5VirtualTable
        CreateFts5VirtualTable --> CreateInsertTrigger
        CreateInsertTrigger --> CreateUpdateTrigger
        CreateUpdateTrigger --> CreateDeleteTrigger
        CreateDeleteTrigger --> CreateIndexes
        CreateIndexes --> CreateStateTable
    }

    state Active {
        [*] --> UpsertFile
        UpsertFile --> Search
        Search --> GetAllFiles
        GetAllFiles --> DeleteFile
        DeleteFile --> GetFileCount
        GetFileCount --> Exec
        Exec --> UpsertFile
    }

    state UpsertFile {
        [*] --> CalculateHash
        CalculateHash --> PrepareStatement
        PrepareStatement --> BindParameters
        BindParameters --> Execute
        Execute --> [*]
    }

    state Search {
        [*] --> BuildQuery
        BuildQuery --> CheckPathsOnly
        CheckPathsOnly --> PrepareStatement
        PrepareStatement --> BindQuery
        BindQuery --> BindLimit
        BindLimit --> IterateResults
        IterateResults --> [*]
    }

    InitSchema --> Active

    state Close {
        [*] --> CheckpointWAL
        CheckpointWAL --> CloseConnection
        CloseConnection --> FreeMemory
        FreeMemory --> [*]
    }

    Active --> Close: deinit()
```

## 4. FTS5 Trigger Synchronization

```mermaid
stateDiagram-v2
    [*] --> FilesTable

    state FilesTable {
        [*] --> INSERT
        INSERT --> TriggerInsert

        state TriggerInsert {
            [*] --> files_ai
            files_ai --> InsertIntoFts5
        }

        INSERT --> UPDATE
        UPDATE --> TriggerUpdate

        state TriggerUpdate {
            [*] --> files_au
            files_au --> DeleteFromFts5: old row
            DeleteFromFts5 --> InsertIntoFts5: new row
        }

        UPDATE --> DELETE
        DELETE --> TriggerDelete

        state TriggerDelete {
            [*] --> files_ad
            files_ad --> DeleteFromFts5
        }
    }

    FilesTable --> FilesFts5

    state FilesFts5 {
        [*] --> AcceptInsert
        AcceptInsert --> IndexContent
        AcceptInsert --> IndexPath
        AcceptDelete --> RemoveContent
    }

    TriggerInsert --> FilesFts5
    TriggerUpdate --> FilesFts5
    TriggerDelete --> FilesFts5
```

## 5. Searcher Flow

```mermaid
stateDiagram-v2
    [*] --> Init

    state Init {
        [*] --> CreateSearcher
        CreateSearcher --> [*]
    }

    state Search {
        [*] --> CheckQueryEmpty
        CheckQueryEmpty --> GetAllFiles: query == ""
        CheckQueryEmpty --> ExecuteSearch: query != ""
        ExecuteSearch --> [*]
    }

    state ExecuteSearch {
        [*] --> CallDbSearch
        CallDbSearch --> CollectResults
        CollectResults --> [*]
    }

    state FormatResults {
        [*] --> CheckJsonOutput
        CheckJsonOutput --> FormatJson: json_output == true
        CheckJsonOutput --> FormatPlain: json_output == false
    }

    state FormatJson {
        [*] --> BeginArray
        BeginArray --> WriteResults
        WriteResults --> EndArray
        EndArray --> WriteNewline
        WriteNewline --> Output
    }

    state FormatPlain {
        [*] --> IterateResults
        IterateResults --> WritePath
        WritePath --> WriteNewline
        WriteNewline --> IterateResults: more results
        IterateResults --> [*]
    }

    FormatResults --> Output

    Output --> [*]
```

## 6. Ignore Rule Matching Flow

```mermaid
stateDiagram-v2
    [*] --> LoadRules

    state LoadRules {
        [*] --> AddVcsIgnore
        AddVcsIgnore --> CheckGitignore
        CheckGitignore --> OpenGitignore: exists
        CheckGitignore --> ReturnRules: not exists
        OpenGitignore --> ReadContent
        ReadContent --> ParseLines
        ParseLines --> CreateRules
        CreateRules --> ReturnRules
        ReturnRules --> [*]
    }

    state MatchPath {
        [*] --> ShouldIgnorePath
        ShouldIgnorePath --> IterateRules

        state IterateRules {
            [*] --> NextRule
            NextRule --> MatchRule
            MatchRule --> CheckResult

            state MatchRule {
                [*] --> CheckDirOnly
                CheckDirOnly --> CheckDirectory: is directory
                CheckDirectory --> MatchPattern
                CheckDirPrefix --> MatchPattern
                CheckDirOnly --> MatchPattern: not dir only
                MatchPattern --> SetIgnored
                SetIgnored --> CheckNegated
                CheckNegated --> Toggle: negated == true
                Toggle --> [*]
            }

            CheckResult --> NextRule: continue
            CheckResult --> ReturnResult: all rules processed
        }

        ReturnResult --> [*]
    }
```

## Component Summary

| Component | States | Transitions | Key Responsibility |
|-----------|--------|-------------|-------------------|
| Main Flow | 8 | 12 | CLI → Operation routing |
| Indexer | 6 | 15 | File discovery → DB upsert |
| Database | 4 | 12 | SQLite lifecycle + FTS5 |
| FTS5 Triggers | 3 | 6 | Automatic index sync |
| Searcher | 4 | 8 | Query → Results → Format |
| Ignore Rules | 2 | 10 | Pattern matching |
