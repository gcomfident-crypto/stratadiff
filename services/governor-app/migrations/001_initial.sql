CREATE TABLE webhook_delivery (
    delivery_id text PRIMARY KEY,
    event_name text NOT NULL,
    installation_id bigint NOT NULL,
    repository_id bigint NOT NULL,
    payload_sha256 text NOT NULL CHECK (char_length(payload_sha256) = 64),
    received_at timestamptz NOT NULL,
    processed_at timestamptz,
    disposition text NOT NULL DEFAULT 'processing'
        CHECK (disposition IN ('processing', 'applied', 'ignored', 'stale'))
);

CREATE TABLE pr_pair (
    id uuid PRIMARY KEY,
    installation_id bigint NOT NULL,
    repository_id bigint NOT NULL,
    repository_full_name text NOT NULL,
    pull_number integer NOT NULL CHECK (pull_number > 0),
    base_sha text NOT NULL CHECK (char_length(base_sha) IN (40, 64)),
    head_sha text NOT NULL CHECK (char_length(head_sha) IN (40, 64)),
    epoch bigint NOT NULL CHECK (epoch > 0),
    fence bigint NOT NULL DEFAULT 0 CHECK (fence >= 0),
    lease_owner text,
    lease_expires_at timestamptz,
    active boolean NOT NULL,
    draft boolean NOT NULL,
    pull_state text NOT NULL CHECK (pull_state IN ('open', 'closed')),
    source_updated_at timestamptz NOT NULL,
    last_delivery_id text NOT NULL REFERENCES webhook_delivery(delivery_id),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    UNIQUE (repository_id, pull_number, base_sha, head_sha),
    UNIQUE (repository_id, pull_number, epoch),
    CHECK (
        (lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX pr_pair_one_active
    ON pr_pair (repository_id, pull_number)
    WHERE active;
CREATE INDEX pr_pair_active_head
    ON pr_pair (repository_id, head_sha)
    WHERE active;

CREATE TABLE dispatch (
    id uuid PRIMARY KEY,
    pair_id uuid NOT NULL REFERENCES pr_pair(id),
    pair_epoch bigint NOT NULL CHECK (pair_epoch > 0),
    provider text NOT NULL,
    command text NOT NULL,
    command_comment_id bigint,
    state text NOT NULL CHECK (state IN ('planned', 'attempting', 'sent', 'abandoned')),
    dispatched_at timestamptz,
    evidence_deadline_at timestamptz,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    UNIQUE (pair_id, pair_epoch, provider),
    CHECK (
        (state = 'sent'
         AND command_comment_id IS NOT NULL
         AND dispatched_at IS NOT NULL
         AND evidence_deadline_at IS NOT NULL)
        OR
        (state <> 'sent' AND evidence_deadline_at IS NULL)
    )
);

CREATE TABLE evidence (
    id uuid PRIMARY KEY,
    pair_id uuid NOT NULL REFERENCES pr_pair(id),
    pair_epoch bigint NOT NULL CHECK (pair_epoch > 0),
    provider text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('issue_comment', 'review', 'status')),
    source_id text NOT NULL,
    source_updated_at timestamptz NOT NULL,
    valid boolean NOT NULL,
    facts jsonb NOT NULL,
    delivery_id text NOT NULL REFERENCES webhook_delivery(delivery_id),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    UNIQUE (pair_id, pair_epoch, kind, source_id)
);
CREATE INDEX evidence_pair_stream
    ON evidence (pair_id, pair_epoch, kind, source_updated_at DESC, source_id DESC);

CREATE TABLE gate_subject (
    id uuid PRIMARY KEY,
    installation_id bigint NOT NULL,
    repository_id bigint NOT NULL,
    repository_full_name text NOT NULL,
    subject_type text NOT NULL CHECK (subject_type IN ('pull_request', 'merge_group')),
    subject_key text NOT NULL,
    pair_id uuid REFERENCES pr_pair(id),
    epoch bigint NOT NULL CHECK (epoch > 0),
    head_sha text NOT NULL CHECK (char_length(head_sha) IN (40, 64)),
    base_sha text NOT NULL CHECK (char_length(base_sha) IN (40, 64)),
    active boolean NOT NULL,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    fence bigint NOT NULL DEFAULT 0 CHECK (fence >= 0),
    lease_owner text,
    lease_expires_at timestamptz,
    desired_state text NOT NULL
        CHECK (desired_state IN ('revoked', 'pending', 'success', 'failure', 'cancelled')),
    desired_summary text NOT NULL,
    check_run_id bigint,
    published_revision bigint,
    published_state text,
    last_delivery_id text NOT NULL REFERENCES webhook_delivery(delivery_id),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    UNIQUE (repository_id, subject_type, subject_key, epoch),
    CHECK (
        (subject_type = 'pull_request' AND pair_id IS NOT NULL)
        OR (subject_type = 'merge_group' AND pair_id IS NULL)
    ),
    CHECK (
        (lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    )
);
CREATE UNIQUE INDEX gate_subject_one_active
    ON gate_subject (repository_id, subject_type, subject_key)
    WHERE active;
CREATE INDEX gate_subject_head
    ON gate_subject (repository_id, head_sha, subject_type)
    WHERE active;

CREATE TABLE outbox (
    id bigserial PRIMARY KEY,
    topic text NOT NULL
        CHECK (topic IN ('publish_gate', 'evaluate_pair', 'dispatch_review', 'reconcile_repository')),
    aggregate_id uuid,
    aggregate_epoch bigint,
    payload jsonb NOT NULL,
    dedupe_key text NOT NULL UNIQUE,
    available_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    lease_owner text,
    lease_expires_at timestamptz,
    lease_fence bigint NOT NULL DEFAULT 0 CHECK (lease_fence >= 0),
    completed_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL,
    CHECK (
        (lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    )
);
CREATE INDEX outbox_ready
    ON outbox (available_at, id)
    WHERE completed_at IS NULL;
