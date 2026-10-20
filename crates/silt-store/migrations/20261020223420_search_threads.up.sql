-- Which thread each search row belongs to, indexed: the FTS table's own
-- columns can't be, so deleting a thread's rows used to scan all of them.
CREATE TABLE search_threads (
    row_id INTEGER PRIMARY KEY,
    thread_id BLOB NOT NULL
);
CREATE INDEX search_threads_by_thread ON search_threads (thread_id);
INSERT INTO search_threads (row_id, thread_id) SELECT rowid, thread_id FROM search;
