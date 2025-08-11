CREATE TABLE tasks (
	id SERIAL PRIMARY KEY,
	owner integer REFERENCES users(id),
	title text,
	description text
)
