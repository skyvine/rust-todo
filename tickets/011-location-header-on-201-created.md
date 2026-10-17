# 011: Every `201 Created` identifies the resource it created

**Phase:** media types, headers, and status codes (after the redesign)
**Depends on:** 005, 006, 010
**Files:** `src/web_api.rs`, `src/core.rs`, `README.md`

## Goal

A client that creates a resource learns the URI of what it created, from the
response, without scraping an id out of a body field.

## Why this comes after the redesign

The obvious first move is to add `Location: /task_by_id?id={id}` and be done.
That value would be obsolete as soon as ticket 006 lands, because
`/task_by_id` is itself replaced by `/tasks/{id}`. With no clients to
transition, the useful order is: land the URIs, then add `Location` pointing at
the final ones.

## Background

RFC 9110 §15.3.2:

> The primary resource created by the request is identified by either a
> Location header field in the response or, if no Location header field is
> received, by the target URI.

Neither creation endpoint identifies its result today:

- `POST /add_task` returns `201 Created` with
  `{"request_id": …, "task_id": 2424}` (`web_api.rs:562-566`). The target URI
  is `/add_task`, which is not the new task, so the fallback clause does not
  apply and the caller is left holding a bare integer.
- `POST /register_account` returns `201 Created` with **an empty body**
  (`HttpResponse::Created().finish()`, `web_api.rs:681`) — not even the id.

Verified against a running server:

```
$ curl -i -X POST …/register_account -d '{"username":"alice","password":"…"}'
HTTP/1.1 201 Created
content-length: 0
date: …
```

`add_user` (`core.rs:359-392`) discards the inserted row, so the handler cannot
know the id at all without a core change.

## In scope

- **The rule**, applied to every `201` in the API after the redesign:
  `POST /tasks` → `Location: /tasks/{id}`;
  `POST /auth_keys` → `Location: /auth_keys/{key}`;
  `PUT` on a share that creates it → `Location: /tasks/{id}/shares/{username}`;
  `POST /users` → `Location: /users/{id}`.
- **`add_user` must report what it created.** Change it in `src/core.rs` to
  return the created user (or at least its id) instead of `()`, keeping the
  `UserExists` → `409` mapping intact. This is the one piece of genuinely
  independent work in the ticket, and it is what makes `POST /users` able to
  answer at all.
- **Return the created resource, not just its id.** `POST /tasks` returns the
  task (the `Task` struct already derives `Serialize`, so it is
  `Task::from_json_object`-compatible); `POST /users` returns a struct with
  `id` and `username`. **Never** return the password hash — note that `User`
  derives `Queryable, Selectable, ZeroizeOnDrop` but not `Serialize`, so a
  response struct is required rather than serializing `User`.
- **Remove the id-only fields.** The bare `task_id` key has no reason to exist
  once the representation is present; delete it rather than keeping it as a
  deprecated alias, since there are no clients reading it.
- `Location` values are built from the id the server stored — a test should
  `GET` the `Location` URI and get the resource back.

## Out of scope

- Whether `POST /users` or `POST /register_account`-style creation should be a
  `PUT` at a client-chosen name. Decided in ticket 010.
- `202 Accepted` for asynchronous work; nothing here is asynchronous.
- Hypermedia links beyond `Location` — `Link` headers pointing at the share
  collection would be a natural follow-up, but a plain `Location` is what the
  spec asks for.

## Acceptance criteria

- [ ] Every `201` in the API carries a `Location` that `GET`s successfully.
- [ ] `POST /tasks` returns the created task; the bare `task_id` key is gone.
- [ ] `POST /users` returns a representation containing `id` and `username`,
      and no password hash.
- [ ] `add_user` returns the created user and still maps a unique violation to
      `409`.
- [ ] No response body anywhere in the API contains a password hash.
- [ ] Tests updated for the moved id: the `add_task` helper and everything that
      reads `task_id` (`extract_json_i32(response, "task_id")` call sites),
      `can_add_task`, `can_get_task`, `regitering_account_is_successful`,
      `duplicate_username_fails`.
- [ ] `README.md` documents `Location` once, as a general rule, rather than
      repeating it per endpoint.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

Ticket 006 and 010 both mention `Location` in their acceptance criteria because
their endpoints are creations and it would be strange to land them knowingly
wrong. If this ticket lands before them, the two should drop that line rather
than duplicate the work — either order is acceptable, but not both
implementations.
