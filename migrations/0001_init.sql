-- 0001_init.sql
-- 初始 schema。详见 docs/storage.md §4。

CREATE TABLE IF NOT EXISTS persona_models (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    archetype TEXT,
    description TEXT,
    current_version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS persona_versions (
    persona_model_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    parent_version INTEGER,
    status TEXT NOT NULL DEFAULT 'building',
    avatar_provider TEXT,
    avatar_provider_version TEXT,
    avatar_primary BLOB,
    avatar_primary_mime TEXT,
    avatar_primary_sha256 TEXT,
    avatar_views_blobs BLOB,
    avatar_refs_blobs BLOB,
    avatar_lora_ref_json TEXT,
    avatar_face_id TEXT,
    voice_provider TEXT,
    voice_provider_version TEXT,
    voice_id_remote TEXT,
    voice_sample BLOB,
    voice_sample_mime TEXT,
    voice_sample_sha256 TEXT,
    voice_transcript TEXT,
    voice_embed BLOB,
    voice_embed_dim INTEGER,
    voice_embed_sha256 TEXT,
    persona_descriptor_json TEXT,
    knowledge_binding_json TEXT,
    anchor_face_emb BLOB,
    anchor_voice_emb BLOB,
    anchor_style_emb BLOB,
    anchor_anchor_sha256 TEXT,
    manifest_json TEXT,
    metrics_json TEXT,
    training_job_id TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (persona_model_id, version),
    FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);

CREATE TABLE IF NOT EXISTS persona_samples (
    id TEXT PRIMARY KEY,
    persona_model_id TEXT NOT NULL,
    version_id_at_collection INTEGER,
    kind TEXT NOT NULL,
    blob BLOB,
    blob_mime TEXT,
    text TEXT,
    source TEXT NOT NULL,
    consent_proof TEXT,
    tags_json TEXT,
    quality_score REAL,
    sha256 TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);
CREATE INDEX IF NOT EXISTS idx_samples_pm ON persona_samples(persona_model_id, kind);

CREATE TABLE IF NOT EXISTS iterate_jobs (
    id TEXT PRIMARY KEY,
    persona_model_id TEXT NOT NULL,
    target_version INTEGER NOT NULL,
    changes_json TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);
CREATE INDEX IF NOT EXISTS idx_iterate_pm ON iterate_jobs(persona_model_id);

CREATE TABLE IF NOT EXISTS finetune_jobs (
    id TEXT PRIMARY KEY,
    persona_model_id TEXT NOT NULL,
    base_version INTEGER NOT NULL,
    target_version INTEGER,
    scope_json TEXT NOT NULL,
    config_json TEXT,
    status TEXT NOT NULL,
    result_version INTEGER,
    drift_report_json TEXT,
    started_at TEXT,
    finished_at TEXT,
    FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);
CREATE INDEX IF NOT EXISTS idx_finetune_pm ON finetune_jobs(persona_model_id);

CREATE TABLE IF NOT EXISTS scripts (
    id TEXT PRIMARY KEY,
    persona_model_id TEXT NOT NULL,
    persona_version INTEGER NOT NULL,
    topic TEXT NOT NULL,
    content_json TEXT,
    duration_ms INTEGER,
    created_at TEXT NOT NULL,
    FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    script_id TEXT,
    persona_model_id TEXT NOT NULL,
    persona_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    progress REAL,
    current_step TEXT,
    options_json TEXT,
    error_json TEXT,
    created_at TEXT,
    finished_at TEXT,
    FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);
CREATE INDEX IF NOT EXISTS idx_jobs_pm ON jobs(persona_model_id);

CREATE TABLE IF NOT EXISTS job_steps (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    status TEXT NOT NULL,
    attempt INTEGER DEFAULT 1,
    outputs_json TEXT,
    error_json TEXT,
    started_at TEXT,
    finished_at TEXT,
    duration_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_steps_job ON job_steps(job_id);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    content BLOB,
    mime TEXT,
    byte_size INTEGER,
    sha256 TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_artifacts_job ON artifacts(job_id, kind);

CREATE TABLE IF NOT EXISTS knowledge_corpora (
    id TEXT PRIMARY KEY,
    name TEXT,
    source_type TEXT,
    language TEXT,
    chunk_count INTEGER DEFAULT 0,
    index_version INTEGER DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS corpus_chunks (
    id TEXT PRIMARY KEY,
    corpus_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    content TEXT NOT NULL,
    embed_blob BLOB,
    embed_dim INTEGER,
    token_count INTEGER,
    deprecated INTEGER DEFAULT 0,
    meta_json TEXT,
    FOREIGN KEY (corpus_id) REFERENCES knowledge_corpora(id)
);
CREATE INDEX IF NOT EXISTS idx_chunks_corpus ON corpus_chunks(corpus_id, ordinal);
