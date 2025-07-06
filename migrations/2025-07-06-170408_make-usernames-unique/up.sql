-- Your SQL goes here
ALTER TABLE users ADD CONSTRAINT usernames_are_unique UNIQUE(username)
