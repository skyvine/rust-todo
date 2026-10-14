Never run `cargo fmt`. The coding style does not currently conform, and running it will create a large amount of noise in the commit.

The test suite requires a running postgresql server. You can create one with the necessary database by running the script at scripts/run\_test\_database.sh. It also requires the environment variable DATABASE\_URL, whose value should look something like `postgres://postgres:test-password@localhost/todo` (this is the correct value for the test database).
