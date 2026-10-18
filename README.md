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
header. RFC 9110 asks every 401 to name the scheme the client may retry with, and lets an
origin server name it on a 403 as well to make clear that authentication succeeded and the
request was still refused, so those endpoints answer a rejected request with
`WWW-Authenticate: Bearer realm="todo"`, plus the RFC 6750 error code which says what to do
next: `error="invalid_request"` when the header was missing or malformed,
`error="invalid_token"` when an auth key was presented but is unknown or expired, and
`error="insufficient_scope"` when the key was accepted but the request is still refused.
Only one challenge is sent, so the realm is sent on its own where no error code applies.
/login, the one endpoint which takes credentials rather than an auth key, answers its own
401s with the Basic challenge instead.

A rejected request is reported as one of two statuses, and the difference tells the client
whether authenticating again could ever help:

- **401 Unauthorized**: the request carried no credentials, or credentials which were never
  accepted. Getting a new auth key is the fix.
- **403 Forbidden**: the request was authenticated and is still refused — reading a task you
  hold no share on, updating one you may only read, or managing the shares on a task you do
  not own. The auth key was good, so a 401 here would send the client in circles asking for
  a key that would be refused exactly the same way.

A task you may not see and a task which does not exist are deliberately indistinguishable:
both answer 403 with the same body shape, on every verb which acts on a task: `GET /tasks/{id}`,
`PATCH /tasks/{id}`, `GET /tasks/{id}/shares`, `PUT /tasks/{id}/shares/{username}`, and
`DELETE /tasks/{id}/shares/{username}`. Usernames and user ids are public, so a real lookup
miss there is still a 404: an unknown target username in a `PUT` or `DELETE` on a share, and
an unknown id in `/user\_by\_id`. A login naming a username no account has is not one of these
misses: it is a refused authentication attempt, and is answered as one.

### Tasks

A task is one resource at one URI, `/tasks/{id}`, and the id is part of the path rather
than a query parameter or a body field, so every operation on a task addresses it the
same way. The collection of tasks the caller can see is the one URI `GET /tasks`, filtered
rather than duplicated: a view of the collection is a query parameter, not a second URI.
Requests and responses are JSON.

Tasks cannot be deleted. There is no `DELETE /tasks/{id}`: a task is created, read, and
changed, and nothing removes it.

- `GET /tasks`: Retrieve the caller's collection of tasks: the tasks they own plus the
  tasks shared with them (read or read-write). The collection is not paginated or sorted;
  every task matching the filter is returned, in the order the database yields them.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Query parameters:
    - completed \[bool] (optional): Return only the tasks whose completed state is this
      value, e.g. `GET /tasks?completed=false` for the incomplete tasks. The parameter
      has the same name and type as the field `Task` serializes it under, so a client
      which knows a task's completion state knows what to send. Leaving it out returns
      the whole collection.
  - Output keys:
    - tasks [array]: The tasks in the collection, narrowed by the filter if one is given.
  - Statuses: `200 OK`; `400` if `completed` is given a value which is not a boolean,
    e.g. `GET /tasks?completed=maybe`, because such a value names no view of the
    collection, with the same JSON error body every other client error carries; `401` if
    the auth key is missing, malformed, unknown, or expired.
- `POST /tasks`: Creates a task.
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
    - title [string]: The title of the new task (should be short)
    - description \[string] (optional): A description of the task (could be long)
  - Output keys:
    - task [object]: The created task.
  - Statuses: `201 Created`, with a `Location: /tasks/{id}` header naming the URI the new
    task can be read and changed at; `400` if the title is unusable; `401` if the auth key
    is missing, malformed, unknown, or expired.
- `GET /tasks/{id}`: Retrieve the contents of the task the URI names. Shared access
  (read or read-write) is honored.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Path parameters:
    - id [number]: The id of the task, e.g. `GET /tasks/1`.
  - Output keys:
    - task [object]: The task with the given id.
  - Statuses: `200 OK`; `404` if the path names no task at all, which includes a missing
    or non-numeric id segment; `403` if the task is not the caller's and not shared with
    them, and for a task which does not exist; `401` if the auth key is missing, malformed,
    unknown, or expired.
- `PATCH /tasks/{id}`: Changes some or all of the data contained in a task. Read-write
  shared access is honored: a user with a `read\_write` share may update the task.
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
    - completed \[bool] (optional): The new completed status of the task.
    - title \[string] (optional): The new title of the task.
    - description \[string] (optional): The new description of the task.
  - Path parameters:
    - id [number]: The id of the task to change, e.g. `PATCH /tasks/1`.
  - Statuses: `200 OK`; `400` if no field at all is provided to change, or if the new
    title is unusable; `403` and `401` as for `GET /tasks/{id}`.

### Shares

A share is one user's access to one task, so it is a sub-resource of the task rather than an
action on it: the shares of a task are at `/tasks/{id}/shares`, and the share for one named
user is at `/tasks/{id}/shares/{username}`. The URI names the share, which is what lets a
client read, change and remove it by URI. Only the owner of a task may list, create or remove
its shares; a `read_write` grantee may edit the task itself but not share it onward.

- `GET /tasks/{id}/shares`: List the shares on the task.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Path parameters:
    - id [number]: The id of the task whose shares to list, e.g. `GET /tasks/1/shares`.
  - Output keys:
    - shares [array]: Each share has `id` [number] (the share's own id, which no other
      resource exposes), `username` [string] and `permission` [string] ("read" or
      "read\_write").
  - Statuses: `200 OK`; `404` if the path names no task at all; `403` if the task is not
    the caller's, and for a task which does not exist; `401` if the auth key is missing,
    malformed, unknown, or expired.
- `PUT /tasks/{id}/shares/{username}`: Assigns a permission to another user on the task,
  creating the share if it is not there and replacing the permission if it is. The URI names
  a fully specified share and the body says what it grants, so this is an idempotent
  assignment and not a submission of a new one: repeating it leaves the share in the same
  state.
  - Input keys:
    - permission [string]: Either "read" or "read\_write". This is the whole
      representation: the task, the sharee, and the URI of the share are all named by the
      path, so there is no `PATCH` on a share and no partial representation to express.
  - Path parameters:
    - id [number]: The id of the task to share, e.g. `PUT /tasks/1/shares/alice`.
    - username [string]: The name of the user to share with.
  - Output keys:
    - share [object]: The share as it now stands, in the same shape the listing uses, so
      a client which gets a 200 learns what the share is without asking again.
  - Statuses: `201 Created`, with a `Location: /tasks/{id}/shares/{username}` header
    naming the share's own URI, when it created the share; `200 OK` when it replaced the
    permission on a share which was already there. The status is the only way to tell those
    two apart, which is why a created share answers 201 with a `Location` and a replaced one
    answers 200 with no `Location` at all. Also `400` if the permission is neither "read"
    nor "read\_write"; `404` if the path names no task, and for a username no account has,
    since usernames are public and a share can name one that does not exist; `403` if the
    task is not the caller's, and for a task which does not exist; `401` if the auth key is
    missing, malformed, unknown, or expired.
- `DELETE /tasks/{id}/shares/{username}`: Removes the share, so the named user loses the
  named access to the named task. This request has no body: the URI says everything it acts
  on, and RFC 9110 §9.3.5 gives content in a `DELETE` no defined meaning, so any
  intermediary which strips such a body would otherwise break the request.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by the login endpoint.
  - Path parameters:
    - id [number]: The id of the task the share is on, e.g. `DELETE /tasks/1/shares/alice`.
    - username [string]: The name of the user whose access to remove.
  - Statuses: `204 No Content`, whether or not the share was there: `DELETE` is idempotent,
    so removing a share which is already gone leaves the state the caller asked for and is
    answered exactly as the removal which got it there was. Also `404` for a username no
    account has; `403` if the task is not the caller's, and for a task which does not
    exist; `401` if the auth key is missing, malformed, unknown, or expired.

### DELETE

- /logout: Disable an auth key
  - Input keys:
    - auth\_key [string]: An auth key as returned by the login endpoint.
- /tasks/{id}/shares/{username}: Remove a user's access to a task owned by the caller. It
  takes no body, and is documented above with the rest of the shares.

### GET

- /is\_alive: unconditionally returns an Ok status
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
    A refused login is refused the same way whether or not the username exists: an unknown username is a refused
    authentication attempt rather than a missing resource, so it answers 401 with the same challenge and the same
    body as a wrong password, never 404. Only an empty or whitespace-only username is a 400, since that is a
    malformed header rather than a refused credential.
  - Output keys:
    - auth\_key [string]: The auth\_key which can be used for requests that require authentication

The task endpoints are documented together, under [Tasks](#tasks); the collection is
`GET /tasks`, and `GET /tasks/{id}`, `POST /tasks`, and `PATCH /tasks/{id}` are the same
resource at one URI. Their shares are documented together under [Shares](#shares), and
`GET /tasks/{id}/shares`, `PUT /tasks/{id}/shares/{username}`, and
`DELETE /tasks/{id}/shares/{username}` are the same share resource at one URI.

### PATCH

There is no `PATCH` endpoint outside the task resource, which is documented above.

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
