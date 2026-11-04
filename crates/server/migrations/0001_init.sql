-- Schema shared by the Rust server and the Next.js website (same tables, same columns).

CREATE TABLE IF NOT EXISTS users (
    id             UUID PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL DEFAULT '',
    password_hash  TEXT NOT NULL,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_ms     BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id          UUID PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    created_ms  BIGINT NOT NULL,
    expires_ms  BIGINT NOT NULL,
    user_agent  TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS sessions_user_idx ON sessions(user_id);

CREATE TABLE IF NOT EXISTS email_tokens (
    token_hash  TEXT PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('verify', 'reset')),
    expires_ms  BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS pages (
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    id          UUID NOT NULL,
    updated_ms  BIGINT NOT NULL,
    deleted     BOOLEAN NOT NULL DEFAULT FALSE,
    data        JSONB NOT NULL DEFAULT 'null',
    PRIMARY KEY (user_id, id)
);
CREATE INDEX IF NOT EXISTS pages_user_updated_idx ON pages(user_id, updated_ms);
