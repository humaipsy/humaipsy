```mermaid
flowchart TB
    subgraph UI
        EDP[Experimental Design Panel]
        Chat[Overview Chat send/receive]
        Play[Play / Trial advance / Timeout]
    end

    subgraph Recording
        ERC[ExperimentCsvRecorder]
        CTX[ExperimentRecordingContext]
    end

    subgraph Inference
        Agent[HumaiAgent]
        Ollama[send_to_ollama]
    end

    EDP -->|Submit / config| ERC
    Play -->|lifecycle events| ERC
    Chat -->|Transcript events| ERC
    Play --> CTX
    Chat --> CTX
    CTX --> Agent
    Agent --> Ollama
    Ollama -->|timing on AI Transcript rows| ERC
```

Experiment CSV files are written to `metrics/` (e.g. `metrics/Exp001.csv`). Settings ping tests do not record to disk.
