# Rust ToDo
Rust ToDo is a REST API for task management written in Rust and using PostgresSQL as a backend.

## Building and Running

Build the image with docker:

```
docker build  --tag rust-todo .
```

Set up a postgres server. The script `scripts/run_test_database` can be used to set one up
locally, but it is insecure (uses a weak password).

Run the image, passing the postgres server's address, username, and password through an
environment variable:

```
docker run -e DATABASE_URL=postgres://postgres:test-password@localhost/todo --network host --rm -it rust-todo
```

## Endpoints

All data is sent and received as JSON. All responses (aside from is\_alive) additionally
include a request\_id key which uniquely identifies the request.

Every endpoint which takes an auth\_key requires it in the `Authorization: Bearer <auth_key>`
header. RFC 9110 asks every 401 to name the scheme the client may retry with, so those
endpoints answer a rejected request with `WWW-Authenticate: Bearer realm="todo"`, plus
the RFC 6750 error code which says what to do next: `error="invalid_request"` when the
header was missing or malformed, and `error="invalid_token"` when an auth key was
presented but is unknown or expired. Only one challenge is sent, so the realm is sent
on its own where no error code applies. `/login`, the one endpoint which takes credentials
rather than an auth key, answers its own 401s with the Basic challenge instead.

### DELETE

- /logout: Disable an auth key
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
- /unshare\_task: Remove a user's access to a task owned by the caller
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to change.
    - username [string]: The username of the user whose access should be removed.

### GET

- /all\_tasks: Retrieve all tasks the caller can see. Tasks shared with the caller (read or read-write) are included.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Output keys:
    - tasks [array]: Tasks owned by the caller plus tasks shared with them.
- /incomplete\_tasks: Retrieve all incomplete tasks the caller can see. Tasks shared with the caller (read or read-write) are included.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Output keys:
    - tasks [array]: Incomplete tasks owned by the caller plus tasks shared with them.
- /is\_alive: unconditionally returns an Ok status
- /task\_by\_id: Retrieve the contents of a task based on its ID. Shared access (read or read-write) is honored.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Query parameters:
    - id [number]: The id of the task to retrieve, e.g. `GET /task\_by\_id?id=1`.
  - Output keys:
    - task [object]: An object that can be deserialized by `Task::from\_json\_object`.
- /task\_shares: List the shares on a task owned by the caller
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Query parameters:
    - id [number]: The id of the task whose shares to list, e.g. `GET /task\_shares?id=1`.
  - Output keys:
    - shares [array]: Each item has `username` [string] and `permission` [string] ("read" or "read\_write").
- /whoami: Retrieves the username of the currently logged in user
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Output keys:
    - username [string]: The name of the user associated with the auth\_key
- /user\_by\_id: Resolve a user id (as found in task `owner` fields and share payloads) to a username
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Query parameters:
    - id [number]: The id of the user to resolve, e.g. `GET /user\_by\_id?id=1`.
  - Output keys:
    - username [string]: The username of the user with the given id.
    - An unknown id returns 404.
### POST

- /register\_account: Creates a new account for future use.
  - Input keys:
    - username [string]: The username for the new account. Must not already exist.
    - password [string]: The password for the new account.
- /login: Generates a new auth key which can be used in future requests.
  - Authentication: HTTP Basic. Send an `Authorization: Basic <base64(username:password)>`
    header and no request body; the username and password are the ones belonging to the account.
  - Failures: Every 401 answer carries a `WWW-Authenticate: Basic realm="todo", charset="UTF-8"`
    challenge, as RFC 9110 requires, whether the header was missing or malformed or the credentials were refused.
    No other endpoint answers with this challenge: they take an auth key, so they name the Bearer scheme instead.
  - Output keys:
    - auth\_key [string]: The auth\_key which can be used for requests that require authentication
- /add\_task: Creates a new task
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
    - title [string]: The title of the new task (should be short)
    - description \[string] (optional): A description of the task (could be long)
  - Output keys:
    - id [number]: A unique identifier for the task.
- /share\_task: Grants another user access to a task owned by the caller, or replaces the permission on an existing share
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to share.
    - username [string]: The username of the user to share with.
    - permission [string]: Either "read" or "read\_write".
### PATCH

- /task\_by\_id: Changes some or all of the data contained in a task. Read-write shared access is honored: a user with a `read\_write` share may update the task.
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to change.
    - completed \[bool] (optional): The new completed status of the task.
    - title \[string] (optional): The new title of the task.
    - description \[string] (optional): The new description of the task.

## Directory Map
- .github/workflows/\*:
    - CI integrations
- http\_requests/\*:
    - Raw HTTP requests useful for manually testing features against a running server
- migrations/\*:
    - Database migrations for use with Diesel
- scripts
    - run\_from\_container.sh:
        - Helper script to run the application from inside a Docker container
    - run\_test\_database.sh:
        - Runs a PostgresSQL database in a docker container with username "postgres" and password "test-password"
        - Creates the todo table
- src
    - core.rs
        - Core application logic for task management; notably, this contains all of the database logic.
    - domain\_types.rs
        - Contains data types which have application-specific logic attached, such as validation on construction or zeroization on drop.
    - lib.rs
        - Entry point for library code
    - main.rs
        - Entry point for running the server
    - schema.rs
        - Generated by Diesel, contains support definitions that map the database schema to rust types
    - web\_api.rs
        - Contains HTTP-specific logic, such as defining endpoints and returning appropriate status codes
