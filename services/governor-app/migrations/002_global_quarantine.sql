ALTER TABLE webhook_delivery
    ALTER COLUMN installation_id DROP NOT NULL,
    ALTER COLUMN repository_id DROP NOT NULL,
    ADD COLUMN scope text NOT NULL DEFAULT 'repository',
    ADD COLUMN error_code text;

ALTER TABLE webhook_delivery
    ADD CONSTRAINT webhook_delivery_scope_check CHECK (
        (scope = 'repository' AND installation_id IS NOT NULL AND repository_id IS NOT NULL)
        OR (scope = 'global' AND installation_id IS NULL AND repository_id IS NULL)
    ),
    ADD CONSTRAINT webhook_delivery_error_code_check CHECK (
        (scope = 'repository' AND error_code IS NULL)
        OR (scope = 'global' AND error_code IN ('invalid_json', 'invalid_repository_envelope'))
    );

CREATE TABLE webhook_delivery_collision (
    delivery_id text NOT NULL REFERENCES webhook_delivery(delivery_id),
    event_name text NOT NULL,
    payload_sha256 text NOT NULL CHECK (char_length(payload_sha256) = 64),
    received_at timestamptz NOT NULL,
    PRIMARY KEY (delivery_id, event_name, payload_sha256)
);

ALTER TABLE pr_pair
    ADD COLUMN quarantined boolean NOT NULL DEFAULT false,
    ADD COLUMN quarantine_delivery_id text REFERENCES webhook_delivery(delivery_id),
    ADD CONSTRAINT pr_pair_quarantine_check CHECK (
        quarantined = (quarantine_delivery_id IS NOT NULL)
    );

ALTER TABLE gate_subject
    ADD COLUMN quarantined boolean NOT NULL DEFAULT false,
    ADD COLUMN quarantine_delivery_id text REFERENCES webhook_delivery(delivery_id),
    ADD CONSTRAINT gate_subject_quarantine_check CHECK (
        quarantined = (quarantine_delivery_id IS NOT NULL)
    );
