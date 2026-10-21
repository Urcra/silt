-- Where to push each person's notifications (ADR 0028): one row per
-- browser that allowed them, as its push service named it.
CREATE TABLE push_subscriptions (
    endpoint TEXT PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);

CREATE INDEX push_subscriptions_user ON push_subscriptions (user_id);
