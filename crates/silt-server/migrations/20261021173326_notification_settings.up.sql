-- Which kinds of notification each person wants notified, not only listed
-- (ADR 0028); kinds without a row take their default.
CREATE TABLE notification_settings (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    notify INTEGER NOT NULL,
    PRIMARY KEY (user_id, kind)
);
