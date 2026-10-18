The test suite requires a running postgresql server. You can create one with the necessary database by running the script at scripts/run\_test\_database.sh. It also requires the environment variable DATABASE\_URL, whose value should look something like `postgres://postgres:test-password@localhost/todo` (this is the correct value for the test database). The test suite uses a single database instance as global state, so do not run multiple instances of the test suite in parallel.

There are currently no clients using this API.

When running `cargo clippy`, always use the `--all-targets` flag.

Do not run any git commands unless explicitly instructed to do so.
