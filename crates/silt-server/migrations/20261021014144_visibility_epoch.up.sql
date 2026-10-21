-- Counts changes to everything visibility is worked out from, so a
-- snapshot of what someone sees is reused until one of them changes.
CREATE TABLE visibility_epoch (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    value INTEGER NOT NULL
);
INSERT INTO visibility_epoch (id, value) VALUES (1, 0);

CREATE TRIGGER visibility_threads_insert AFTER INSERT ON threads BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_threads_update AFTER UPDATE ON threads BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_threads_delete AFTER DELETE ON threads BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_families_insert AFTER INSERT ON families BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_families_update AFTER UPDATE ON families BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_families_delete AFTER DELETE ON families BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_projects_insert AFTER INSERT ON projects BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_projects_update AFTER UPDATE ON projects BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_projects_delete AFTER DELETE ON projects BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_orgs_insert AFTER INSERT ON orgs BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_orgs_update AFTER UPDATE ON orgs BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_orgs_delete AFTER DELETE ON orgs BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_org_members_insert AFTER INSERT ON org_members BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_org_members_update AFTER UPDATE ON org_members BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_org_members_delete AFTER DELETE ON org_members BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_teams_insert AFTER INSERT ON teams BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_teams_update AFTER UPDATE ON teams BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_teams_delete AFTER DELETE ON teams BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_team_members_insert AFTER INSERT ON team_members BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_team_members_update AFTER UPDATE ON team_members BEGIN UPDATE visibility_epoch SET value = value + 1; END;
CREATE TRIGGER visibility_team_members_delete AFTER DELETE ON team_members BEGIN UPDATE visibility_epoch SET value = value + 1; END;
