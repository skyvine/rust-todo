# Rust ToDo
Rust ToDo is a REST API for task management written in Rust and using PostgresSQL as a backend.

## Endpoints

All data is sent and received as JSON. All responses (aside from is_alive) additionally
include a request_id key which uniquely identifies the request.

### DELETE

- /logout: Disable an auth key
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
- /unshare_task: Remove a user's access to a task owned by the caller
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to change.
    - username [string]: The username of the user whose access should be removed.

### GET

- /all_tasks: Retrieve all tasks the caller can see. Tasks shared with the caller (read or read-write) are included.
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
  - Output keys:
    - tasks [array]: Tasks owned by the caller plus tasks shared with them.
- /incomplete_tasks: Retrieve all incomplete tasks the caller can see. Tasks shared with the caller (read or read-write) are included.
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
  - Output keys:
    - tasks [array]: Incomplete tasks owned by the caller plus tasks shared with them.
- /is_alive: unconditionally returns an Ok status
- /task_by_id: Retrieve the contents of a task based on its ID. Shared access (read or read-write) is honored.
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to retrieve.
  - Output keys:
    - task [object]: An object that can be deserialized by `Task::from_json_object`.
- /task_shares: List the shares on a task owned by the caller
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task whose shares to list.
  - Output keys:
    - shares [array]: Each item has `username` [string] and `permission` [string] ("read" or "read_write").
- /whoami: Retrieves the username of the currently logged in user
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
  - Output keys:
    - username [string]: The name of the user associated with the auth_key
- /user_by_id: Resolve a user id (as found in task `owner` fields and share payloads) to a username
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the user to resolve.
  - Output keys:
    - username [string]: The username of the user with the given id.
    - An unknown id returns 404.
### POST

- /register_account: Creates a new account for future use.
  - Input keys:
    - username [string]: The username for the new account. Must not already exist.
    - password [string]: The password for the new account.
- /login: Generates a new auth key which can be used in future requests.
  - Input keys:
    - username [string]: The username to log in with.
    - password [string]: The password associated with the user
  - Output keys:
    - auth_key [string]: The auth_key which can be used for requests that require authentication
- /add_task: Creates a new task
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - title [string]: The title of the new task (should be short)
    - description \[string] (optional): A description of the task (could be long)
  - Output keys:
    - id [number]: A unique identifier for the task.
- /task_by_id: Changes some or all of the data contained in a task. Read-write shared access is honored: a user with a `read_write` share may update the task.
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to change.
    - completed \[bool] (optional): The new completed status of the task.
    - title \[string] (optional): The new title of the task.
    - description \[string] (optional): The new description of the task.
- /share_task: Grants another user access to a task owned by the caller, or replaces the permission on an existing share
  - Input keys:
    - auth_key [string]: An auth key as returned by the login endpoint.
    - id [number]: The id of the task to share.
    - username [string]: The username of the user to share with.
    - permission [string]: Either "read" or "read_write".

## Directory Map
- .github/workflows/*:
    - CI integrations
- http_requests/*:
    - Raw HTTP requests useful for manually testing features against a running server
- migrations/*:
    - Database migrations for use with Diesel
- scripts
    - run_from_container.sh:
        - Helper script to run the application from inside a Docker container
    - run_test_database.sh:
        - Runs a PostgresSQL database in a docker container with username "postgres" and password "test-password"
        - Creates the todo table
- src
    - core.rs
        - Core application logic for task management; notably, this contains all of the database logic.
    - domain_types.rs
        - Contains data types which have application-specific logic attached, such as validation on construction or zeroization on drop.
    - lib.rs
        - Entry point for library code
    - main.rs
        - Entry point for running the server
    - schema.rs
        - Generated by Diesel, contains support definitions that map the database schema to rust types
    - web_api.rs
        - Contains HTTP-specific logic, such as defining endpoints and returning appropriate status codes
