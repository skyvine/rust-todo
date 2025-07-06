CREATE TABLE auth_keys (
	id SERIAL PRIMARY KEY,
	user_id integer REFERENCES users NOT NULL,
	key text NOT NULL,
	expiration timestamp NOT NULL DEFAULT (now() + interval '30' day)
)
