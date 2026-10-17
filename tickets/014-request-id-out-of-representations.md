# 014: Return resources directly, and move `request_id` to a header

**Phase:** media types, headers, and status codes (after the redesign)
**Depends on:** 001, 011, 006–010
**Files:** `src/web_api.rs`, `README.md`

## Goal

A URI returns the representation of its own resource, response metadata travels
in headers, and a success response is not built by a function named for errors.

## Why this comes after the redesign

Both halves of this change every response body in the API. Doing it against the
old URIs means touching every handler and test once, then touching them all
again when the URIs change. With no clients, there is no reason to pay that
twice.

## Background

Every response body is wrapped in an envelope carrying `request_id`
(`README.md` lines 24-25). Three separate problems:

- **No URI returns the representation of its resource.**
  `GET /task_by_id?id=1` returns `{"request_id": …, "task": {…}}`, so the task
  is reachable only by knowing to unwrap `"task"`. Same for `"tasks"`,
  `"shares"`, and `"username"`. Fielding's constraint 6 expects a URI to yield
  a representation of the resource it names.
- **The error-body helper is the success-body helper.**
  `error_body(request_id)` (`web_api.rs:26-28`) builds the successful responses
  of `share_task` (`web_api.rs:739`), `update_task_by_id` (`web_api.rs:831`),
  and `unshare_task` (`web_api.rs:884`). The response is `200 OK` with a body
  shaped exactly like a failure, distinguished only by the status code.
- **`request_id` is response metadata in a representation.** The existing spans
  already emit it (`span!(Level::ERROR, "Add Task", %request_id)`); it is
  useful for correlating a failure with logs, and a header is where it belongs.

This ticket previously deferred the envelope removal to ticket 006 on the
grounds that doing both at once is more disruptive. There are no clients, so
the disruption argument does not apply and the two halves are merged here.

## In scope

- **Return the resource directly.** After the redesign:
  - `GET /tasks/{id}` → the task object, not `{"task": …}`
  - `GET /tasks` → a JSON array of task objects, not `{"tasks": […]}` with a
    single-element array per envelope
  - `GET /tasks/{id}/shares` → an array of share objects
  - `GET /users/me`, `GET /users/{id}` → the user object (`id`, `username`)
  - `POST /auth_keys` → the auth key representation
- **Send `request_id` as a header on every response**, including the ones built
  by the extractors (`unauthorized_error`, `unauthorized_basic_error`,
  `query_error_handler`) and the 415/400s actix generates for bad payloads. Use
  a conventional name — `X-Request-Id`, or W3C `traceparent` if trace
  propagation is wanted. Pick one and document it.
  - **Decision**: Use `traceparent`.
- **Remove the `request_id` body key entirely.** No deprecation window: a
  deprecated field is still a field to maintain, and there is nobody to
  transition.
- **Return `204 No Content` from mutations with nothing to report**:
  `PUT /tasks/{id}/shares/{username}` when replacing an existing permission,
  `DELETE /tasks/{id}/shares/{username}`, `PATCH /tasks/{id}`, and
  `DELETE /auth_keys`. If a mutation genuinely has something to say, return the
  resource.
- **Separate the helpers.** Add `json_body` / `no_content` next to
  `error_body` / `error_body_with_message`, so no success path calls an error
  helper.
- Check that `InternalError::from_response("Missing or malformed Authorization
  header", …)` cannot leak its cause string into the response body — the body
  must be the JSON error body only.

## Out of scope

- The error-body envelope itself (`{"request_id": …, "message": …}`). Error
  bodies are not resource representations, and a consistent error shape is
  worth keeping; only the duplicated `request_id` key is affected. Move it to
  the header there too.
- The status codes those bodies carry — fixed by tickets 003 and 004.
- Hypermedia (`Link` headers, `HAL`, `JSON:API`-style documents). This ticket
  makes the representation the resource; it does not add links between
  resources.

## Acceptance criteria

- [ ] Every URI returns its resource directly — no wrapping key.
- [ ] Every response, including framework-generated errors, carries the request
      id header.
- [ ] No response body contains a `request_id` key.
- [ ] `error_body` / `error_body_with_message` are not called from any success
      path.
- [ ] The four mutations listed above return `204 No Content`.
- [ ] `assert_json_error_with_request_id` (`web_api.rs:2510-2516`) is
      complemented by a header assertion, since that is now where the id comes
      from.
- [ ] Tests updated for the shape change: `extract_json_tasks`,
      `extract_shares`, `extract_json_from_constructor`, and every call site
      that indexed a wrapper key.
- [ ] `README.md` documents the request id header once and states that
      responses are the resource, with no envelope.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

Returning an array from `GET /tasks` rather than an envelope is the change
most likely to surprise, and it is the correct one: a collection URI returning a
collection.

Since this removes fields rather than renaming them, it is the one ticket in
the set that cannot be partially landed. Do it whole.
