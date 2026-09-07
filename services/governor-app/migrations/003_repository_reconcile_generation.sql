CREATE TABLE repository_reconcile_generation (
    repository_id bigint PRIMARY KEY,
    generation bigint NOT NULL CHECK (generation > 0),
    updated_at timestamptz NOT NULL
);

INSERT INTO repository_reconcile_generation (repository_id, generation, updated_at)
SELECT repository_id, 1, MAX(updated_at)
FROM (
    SELECT repository_id, updated_at FROM pr_pair
    UNION ALL
    SELECT repository_id, updated_at FROM gate_subject
) AS repositories
GROUP BY repository_id
ON CONFLICT (repository_id) DO NOTHING;
