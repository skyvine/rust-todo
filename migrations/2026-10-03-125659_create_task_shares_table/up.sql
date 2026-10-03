CREATE TABLE task_shares (
    id SERIAL PRIMARY KEY,
    task_id integer NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    user_id integer NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permission text NOT NULL CHECK (permission IN ('read', 'read_write')),
    UNIQUE (task_id, user_id)
);
