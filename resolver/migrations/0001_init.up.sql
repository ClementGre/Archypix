-- Resolver base schema (converted from the old init_database bootstrap, feature 23 §2.7).

CREATE TABLE backends
(
    back_domain  VARCHAR(255) PRIMARY KEY, -- public host[:port]
    use_https    BOOLEAN      NOT NULL,    -- scheme for the public URL
    internal_url VARCHAR(255) NOT NULL,    -- how the resolver reaches this backend
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT now()
);

CREATE TABLE user_mappings
(
    username    VARCHAR(255) PRIMARY KEY,
    back_domain VARCHAR(255) NOT NULL REFERENCES backends (back_domain),
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT now()
);

CREATE INDEX idx_user_mappings_back_domain ON user_mappings (back_domain);
