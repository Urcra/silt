DROP TABLE projects;
DROP TABLE families;
ALTER TABLE threads DROP COLUMN owner_id;
ALTER TABLE pairing_codes DROP COLUMN user_id;
ALTER TABLE devices DROP COLUMN user_id;
DROP TABLE users;
