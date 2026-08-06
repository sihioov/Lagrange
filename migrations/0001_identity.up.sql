-- 0001: Identity — users, roles, user_roles, invitations, web_sessions.
-- Design §7.2 (Identity); requirements §3, §10.3 NFR-SEC-004; plan Todo 3.
-- Invitations and web_sessions are tenant tables (ownership via user_id);
-- users/roles/user_roles are system-owned shared tables, read-only to serving
-- roles (grants land in 0009).

CREATE TABLE users (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    issuer       text NOT NULL,                -- OIDC issuer, immutable identity binding
    subject      text NOT NULL,                -- OIDC subject, immutable identity binding
    email        text NOT NULL,                -- normalized, single-use per invite
    display_name text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT users_issuer_subject_key UNIQUE (issuer, subject),
    CONSTRAINT users_email_key UNIQUE (email)
);

CREATE TABLE roles (
    id          text PRIMARY KEY,              -- 'owner' | 'member'
    description text NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE user_roles (
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id    text NOT NULL REFERENCES roles (id),
    granted_by uuid REFERENCES users (id),
    granted_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, role_id)
);

-- Invitations are created by the Owner (user_id) for a prospective Member.
-- The single-use token is never stored; only its sha256 is kept.
CREATE TABLE invitations (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id             uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email               text NOT NULL,
    invite_hash         text NOT NULL,
    status              text NOT NULL DEFAULT 'PENDING',
    expires_at          timestamptz NOT NULL,
    redeemed_by_user_id uuid REFERENCES users (id),
    redeemed_at         timestamptz,
    created_at          timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT invitations_invite_hash_check CHECK (invite_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT invitations_status_check CHECK (status IN ('PENDING', 'REDEEMED', 'REVOKED', 'EXPIRED')),
    CONSTRAINT invitations_invite_hash_key UNIQUE (invite_hash)
);
CREATE INDEX invitations_user_id_idx ON invitations (user_id);
CREATE INDEX invitations_email_idx ON invitations (email);

-- First-party opaque sessions (Todo 22 SessionStore seam): the random
-- __Host-lagrange_session cookie value is hashed with sha256 before storage;
-- the CSRF synchronizer token is stored as a sha256 hash too.
CREATE TABLE web_sessions (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    session_hash text NOT NULL,
    csrf_hash    text NOT NULL,
    expires_at   timestamptz NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    revoked_at   timestamptz,
    CONSTRAINT web_sessions_session_hash_check CHECK (session_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT web_sessions_csrf_hash_check CHECK (csrf_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT web_sessions_expiry_check CHECK (expires_at > created_at),
    CONSTRAINT web_sessions_session_hash_key UNIQUE (session_hash)
);
CREATE INDEX web_sessions_user_id_idx ON web_sessions (user_id);
