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

All data is sent and received as JSON. All responses (aside from `/health`) additionally
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
`POST /auth_keys`, the one endpoint which takes credentials rather than an auth key, answers
its own 401s with the Basic challenge instead.

A rejected request is reported as one of two statuses, and the difference tells the client
whether authenticating again could ever help:

- **401 Unauthorized**: the request carried no credentials, or credentials which were never
  accepted. Getting a new auth key is the fix.
- **403 Forbidden**: the request was authenticated and is still refused — reading a task you
  hold no share on, updating one you may only read, or managing the shares on a task you do
  not own. The auth key was good, so a 401 here would send the client in circles asking for
  a key that would be refused exactly the same way.

A path the server knows and a method it serves are two different things. A request for a
method a path does not support is answered `405 Method Not Allowed`, with an `Allow` header
naming the methods that path does support: `GET, POST` on the task collection, `GET` on a
user, and so on. RFC 9110 §15.5.6 asks for exactly this — the resource is there, so a `404`
would claim it is not, and the `Allow` header says which verb to use instead. A path the
server does not know at all is still a `404`. The `405` carries the same JSON error body as
every other client error.

A task you may not see and a task which does not exist are deliberately indistinguishable:
both answer 403 with the same body shape, on every verb which acts on a task: `GET /tasks/{id}`,
`PATCH /tasks/{id}`, `GET /tasks/{id}/shares`, `PUT /tasks/{id}/shares/{username}`, and
`DELETE /tasks/{id}/shares/{username}`. Usernames and user ids are public, so a real lookup
miss there is still a 404: an unknown target username in a `PUT` or `DELETE` on a share, and
an unknown id in `/users/{id}`. `POST /auth_keys` naming a username no account has is not one
of these misses: it is a refused authentication attempt, and is answered as one.

Every endpoint which creates a resource answers `201 Created` with the created resource twice
over: a `Location` header naming the URI of the resource it just created, and the resource
itself in the body. RFC 9110 §15.3.2 identifies the primary resource created by a request
either by the `Location` header or, failing it, by the target URI — and the target URI of a
creation is the collection the resource was created in, not the new member of it, so the header
is what names what was made. The `Location` is a relative reference (`/tasks/1`), never the
whole URL, and never a secret: no `Location` in this API carries a credential.

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
    where it is an auth key as returned by `POST /auth_keys`.
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
    the auth key is missing, malformed, unknown, or expired; `405` with an
    `Allow: GET, POST` header, for any other method on this path.
- `POST /tasks`: Creates a task.
  - Input keys:
    - auth\_key [string]: An auth key as returned by `POST /auth_keys`.
    - title [string]: The title of the new task (should be short)
    - description \[string] (optional): A description of the task (could be long)
  - Output keys:
    - task [object]: The created task.
  - Statuses: `201 Created`, with a `Location: /tasks/{id}` header naming the URI the new task
    can be read and changed at; `400` if the title is unusable; `401` if the auth key is
    missing, malformed, unknown, or expired; `405` with an `Allow: GET, POST` header, for
    any other method on this path.
- `GET /tasks/{id}`: Retrieve the contents of the task the URI names. Shared access
  (read or read-write) is honored.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by `POST /auth_keys`.
  - Path parameters:
    - id [number]: The id of the task, e.g. `GET /tasks/1`.
  - Output keys:
    - task [object]: The task with the given id.
  - Statuses: `200 OK`; `404` if the path names no task at all, which includes a missing
    or non-numeric id segment; `403` if the task is not the caller's and not shared with
    them, and for a task which does not exist; `401` if the auth key is missing, malformed,
    unknown, or expired; `405` with an `Allow: GET, PATCH` header, for any other method on
    this path.
- `PATCH /tasks/{id}`: Changes some or all of the data contained in a task. Read-write
  shared access is honored: a user with a `read\_write` share may update the task.
  - Input keys:
    - auth\_key [string]: An auth key as returned by `POST /auth_keys`.
    - completed \[bool] (optional): The new completed status of the task.
    - title \[string] (optional): The new title of the task.
    - description \[string] (optional): The new description of the task.
  - Path parameters:
    - id [number]: The id of the task to change, e.g. `PATCH /tasks/1`.
  - Statuses: `200 OK`; `400` if no field at all is provided to change, or if the new
    title is unusable; `403` and `401` as for `GET /tasks/{id}`; `405` with an
    `Allow: GET, PATCH` header, for any other method on this path.

### Shares

A share is one user's access to one task, so it is a sub-resource of the task rather than an
action on it: the shares of a task are at `/tasks/{id}/shares`, and the share for one named
user is at `/tasks/{id}/shares/{username}`. The URI names the share, which is what lets a
client change and remove it by URI. Only the owner of a task may list, create or remove
its shares; a `read_write` grantee may edit the task itself but not share it onward.

The shares of a task are read as the collection at `GET /tasks/{id}/shares`. Nothing is served
at the URI of a single share, so the `Location` a created share carries names a URI which does
not answer.

- `GET /tasks/{id}/shares`: List the shares on the task.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by `POST /auth_keys`.
  - Path parameters:
    - id [number]: The id of the task whose shares to list, e.g. `GET /tasks/1/shares`.
  - Output keys:
    - shares [array]: Each share has `id` [number] (the share's own id, which no other
      resource exposes), `username` [string] and `permission` [string] ("read" or
      "read\_write").
  - Statuses: `200 OK`; `404` if the path names no task at all; `403` if the task is not
    the caller's, and for a task which does not exist; `401` if the auth key is missing,
    malformed, unknown, or expired; `405` with an `Allow: GET` header, for any other method
    on this path.
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
  - Statuses: `201 Created`, with a `Location: /tasks/{id}/shares/{username}` header naming
    the share's own URI, when it created the share; `200 OK`, with no `Location` at all, when
    it replaced the permission on a share which was already there. The status is the only way
    to tell those two apart. Also `400` if the permission is neither "read" nor
    "read\_write"; `404` if the path names no task, and for a username no account has, since
    usernames are public and a share can name one that does not exist; `403` if the task is
    not the caller's, and for a task which does not exist; `401` if the auth key is missing,
    malformed, unknown, or expired; `405` with an `Allow: PUT, DELETE` header, for any other
    method on this path.
- `DELETE /tasks/{id}/shares/{username}`: Removes the share, so the named user loses the
  named access to the named task. This request has no body: the URI says everything it acts
  on, and RFC 9110 §9.3.5 gives content in a `DELETE` no defined meaning, so any
  intermediary which strips such a body would otherwise break the request.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by `POST /auth_keys`.
  - Path parameters:
    - id [number]: The id of the task the share is on, e.g. `DELETE /tasks/1/shares/alice`.
    - username [string]: The name of the user whose access to remove.
  - Statuses: `204 No Content`, whether or not the share was there: `DELETE` is idempotent,
    so removing a share which is already gone leaves the state the caller asked for and is
    answered exactly as the removal which got it there was. Also `404` for a username no
    account has; `403` if the task is not the caller's, and for a task which does not
    exist; `401` if the auth key is missing, malformed, unknown, or expired; `405` with an
    `Allow: PUT, DELETE` header, for any other method on this path.

### Users

A user is an account, and it is a resource at one URI, `/users/{id}`, where the id is the
one which appears in the `owner` field of a task and in the share payloads. User ids are
public, so a user can be read by anyone who knows its id.

The caller's own user is at `/users/me`, a URI which names the caller rather than a row.
It is a special case of `/users/{id}` and is matched first, so `me` is never read as an
id. The id is in the answer as well as the username, because it appears in every task
and share payload, and a client which could not look its own id up would have to guess
it.

There is no listing of users. A client which has an id can read that user, and one which
has only a username has nothing to look up, because the API never offered a lookup by
name.

- `POST /users`: Creates an account. The URI names the collection users are created in,
  not an operation to perform on it.
  - Input keys:
    - username [string]: The username for the new account. Must not already exist.
    - password [string]: The password for the new account.
  - Output keys:
    - user [object]: The created user, with its `id` [number] and its `username`
      [string]. No response ever carries the password or its hash.
  - Statuses: `201 Created`, with a `Location: /users/{id}` header naming the URI the
    new user can be read at; `400` if the username is unusable; `409` if the username is
    already taken; `405` with an `Allow: POST` header, for any other method on this path.
- `GET /users/me`: Retrieves the caller's own user: the resource relative to the caller.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by `POST /auth_keys`.
  - Output keys:
    - user [object]: The caller's user, with its `id` [number] and its `username`
      [string].
  - Statuses: `200 OK`; `401` if the auth key is missing, malformed, unknown, or expired;
    `405` with an `Allow: GET` header, for any other method on this path.
- `GET /users/{id}`: Retrieves the user the URI names.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header,
    where it is an auth key as returned by `POST /auth_keys`.
  - Path parameters:
    - id [number]: The id of the user to retrieve, e.g. `GET /users/1`.
  - Output keys:
    - user [object]: The user with the given id, with its `id` [number] and its
      `username` [string].
  - Statuses: `200 OK`; `404` if the path names no user at all, which includes a
    missing or non-numeric id segment, and an id no account has, because user ids are
    public; `401` if the auth key is missing, malformed, unknown, or expired; `405` with an
    `Allow: GET` header, for any other method on this path.

### Auth keys

An auth key is a secret the API issues to a caller and accepts back on every later request,
so it is a resource at one URI, `/auth_keys`, rather than a login and a logout. The API is
stateless: there is no session and no cookie. The key is presented in the
`Authorization: Bearer <auth_key>` header like any other credential, and a client which wants
to stop using one revokes it.

Credentials are still exchanged with HTTP Basic. `POST /auth_keys` is the only endpoint which
takes a username and a password; every other endpoint takes an auth key and answers its 401s
with the Bearer challenge instead.

- `POST /auth_keys`: Creates an auth key for the account the credentials name. This is what
  logging in is: the credentials go out in the header and never in a body, and what comes back
  is a new key.
  - Takes no body. Authentication: HTTP Basic. Send an
    `Authorization: Basic <base64(username:password)>` header; the username and password are
    the ones belonging to the account.
  - Output keys:
    - auth\_key [string]: The auth key, which the client sends as
      `Authorization: Bearer <auth_key>` on every endpoint which needs one.
  - Statuses: `201 Created`, with a `Location: /auth_keys/{id}` header naming the URI of the
    key just created and the key itself as its representation, so a client never has to ask a
    second question to learn what it was handed. The `Location` carries the id of the key row
    and never the key value: an auth key is a bearer token, and RFC 6750 §5.3 keeps those out
    of URLs, because every proxy and log which records a request line would record the
    credential. Every 401 answer carries a
    `WWW-Authenticate: Basic realm="todo", charset="UTF-8"` challenge, as RFC 9110 requires,
    whether the header was missing or malformed or the credentials were refused; no other
    endpoint answers with this challenge, because they take an auth key and so name the Bearer
    scheme instead. A refused attempt is refused the same way whether or not the username
    exists: an unknown username is a refused authentication attempt rather than a missing
    resource, so it answers 401 with the same challenge and the same body as a wrong password,
    never 404. Only an empty or whitespace-only username is a `400`, since that is a malformed
    header rather than a refused credential; `405` with an `Allow: POST, DELETE` header, for
    any other method on this path.
- `DELETE /auth_keys`: Revokes the auth key the caller presented, so it stops working
  immediately.
  - Takes no body. Send the auth key in the `Authorization: Bearer <auth_key>` header, where it
    is an auth key as returned by `POST /auth_keys`.
  - Statuses: `204 No Content`, whether or not the key was there: `DELETE` is idempotent, so
    revoking a key which is already gone leaves the state the caller asked for and is answered
    exactly as the removal which got it there was. A key which has been revoked answers `401`
    with the bearer challenge on every endpoint which takes one; `405` with an
    `Allow: POST, DELETE` header, for any other method on this path.

There is no `GET` on an auth key and no `DELETE /auth_keys/{id}`, so nothing is served at the
URI the `Location` above names. Revoking a key is done by presenting it in the header, which is
why `DELETE /auth_keys` needs no URI segment at all. A revoked key stops working at once rather
than at some later expiry, and nothing refreshes or rotates it.

### Health

`GET /health` is a liveness probe rather than a resource: it says whether the server is up
and adds nothing else. `/health` is the name every deployment tool already understands,
where `/is_alive` read like a boolean field rather than an endpoint.

It is the one endpoint which takes no credentials, because whatever is watching the server
has no auth key to present, and a probe which asks for one would only report that the
watcher is unauthenticated.

- `GET /health`: Answers `200 OK` with an empty body, for as long as the server is running.
  - Takes no body and no credentials.
  - Statuses: `200 OK`; `405` with an `Allow: GET` header, for any other method on this
    path.

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
