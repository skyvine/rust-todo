# 006: Make a task one resource at one URI

**Phase:** the redesign
**Depends on:** 005
**Files:** `src/web_api.rs`, `src/lib.rs`, `README.md`, `http_requests/`

## Goal

A task is addressed by its own URI. `POST /tasks`, `GET /tasks/{id}` and
`PATCH /tasks/{id}` replace the three-URIs-one-resource arrangement around
`/add_task` and `/task_by_id`.

## Background

Today one task resource lives at three URIs:

- `POST /add_task` creates it,
- `GET /task_by_id?id=N` reads it,
- `PATCH /task_by_id` with `id` in the body updates it.

Fielding's constraint 3 requires the URI to identify the resource, and
constraint 6 requires that retrieving a URI return a representation of that
resource. Neither holds: the id is a query parameter on one verb and a body
field on another, so no single URI names the task. Practically, this also
means no task URI is cacheable, linkable, or referenceable, and a client that
wants to update a task must construct an identifier into a body instead of
addressing the resource.

## In scope

- `POST /tasks` — body `{"title", "description"?}` as today. Returns `201` with
  `Location: /tasks/{id}` and the created task as its representation. (Ticket 011
  owns the general `Location` rule and the removal of the bare `task_id` key;
  if it lands first, drop that line here rather than duplicating the work.)
- `GET /tasks/{id}` — path parameter via `web::Path<i32>` or
  `web::Path<TaskPath>`. Drops the `TaskByIdQuery` struct
  (`web_api.rs:336-339`) and the `?id=` requirement. A non-numeric or missing
  path segment yields `404` from the router rather than `400` from a query
  extractor — that is the right status for a path that cannot name a resource,
  and the `query_error_handler` no longer applies to these routes.
- `PATCH /tasks/{id}` — body contains only the fields to change. `id` leaves
  `UpdateTaskPayload` (`web_api.rs:769-774`); the `#[serde(flatten)]` of
  `TaskUpdates` stays, since it keeps the "no fields provided" check
  (`web_api.rs:790-796`) working without enumerating fields.
- Tasks are not deletable, say so in the README rather than leaving `DELETE` to
  fall through to the 404 from ticket 012.
- With ticket 014's envelope removal, `GET /tasks/{id}` returns the task
  representation directly rather than `{"request_id": …, "task": {…}}`. Ticket
  014 is sequenced after this one, so this ticket keeps the existing envelope
  shape and 014 changes it once for the whole API.
- Core logic (`add_task`, `get_task_by_id`, `update_task`) is unchanged apart
  from what these handlers pass in.

## Out of scope

- Task listing and filtering (ticket 007).
- Shares (ticket 008).
- Query-string filtering on the task collection (`GET /tasks?completed=`), which
  ticket 007 introduces.
- Changing the sharing or permission rules.

## Acceptance criteria

- [ ] `POST /tasks`, `GET /tasks/{id}`, `PATCH /tasks/{id}` exist and the old
      `/add_task` and `/task_by_id` routes are gone.
- [ ] No task identifier is read from a query string or request body any more.
- [ ] `POST /tasks` returns `201` with `Location: /tasks/{id}`; a `GET` on that
      `Location` returns the task.
- [ ] A non-numeric or missing id in the path returns `404`, not `400`.
- [ ] `PATCH /tasks/{id}` with an empty body is still `400` ("at least one
      field must be provided").
- [ ] Tests updated for the new URIs: `can_add_task`,
      `cannot_add_task_without_valid_auth_key`, `can_get_task`,
      `cannot_get_different_users_task`, `cannot_get_nonexistent_task`,
      `cannot_get_task_without_a_valid_id_query_parameter` (repurpose or
      delete — there is no query string to reject), `cannot_update_nonexistent_task`,
      `title_is_updatable`, `description_is_updatable`, `completed_is_updatable`,
      `update_modifies_only_the_target_task`, the `update_task` helper,
      `read_write_sharee_can_update_task_fields`,
      `read_only_sharee_cannot_update_task`, `unrelated_user_cannot_update_task`,
      `read_share_grants_get_and_listing_access`, `no_share_means_401_and_no_listing_entry`.
- [ ] `README.md` has one task section instead of the `add_task`/`task_by_id`
      split, documenting the media type, statuses, and path parameter.
- [ ] `http_requests/` updated: `patch_task_by_id.http` replaced, an add-task
      request added.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

This is the largest single test-churn ticket in the redesign, since most tests
address tasks. Doing it before 007 and 008 keeps the churn spread out: each of
those touches a smaller set of tests.

The `#[ctor] global_init` wipes all tables before the suite runs, so ids are
not stable across tests; keep using the id returned by `add_task` rather than
hard-coding one.
