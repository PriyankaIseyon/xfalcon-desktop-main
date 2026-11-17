-- Which trusted project a saved report (a DA kit) was created in. DA keeps no
-- such link for a kit an assistant saves, so the device records it when the
-- save call returns. Device-local, like conversation_project_bindings: the
-- first origin wins and a removed project takes its links with it.
CREATE TABLE IF NOT EXISTS report_kit_projects (
    account_id TEXT NOT NULL,
    kit_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    PRIMARY KEY (account_id, kit_id),
    FOREIGN KEY (account_id, project_id)
        REFERENCES trusted_projects (account_id, project_id)
        ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_report_kit_projects_project
    ON report_kit_projects (account_id, project_id);
