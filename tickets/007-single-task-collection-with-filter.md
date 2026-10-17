# 007: One task collection with a filter, not two URIs

**Phase:** the redesign
**Depends on:** 005, 006
**Files:** `src/web_api.rs`, `src/core.rs`, `README.md`, `http_requests/`

## Goal

`/all_tasks` and `/incomplete_tasks` become one collection URI with a filter,
so one resource is reachable at one URI.

## Background

Fielding's constraint 4 states that the URI must identify a resource, and the
same resource must not be reachable under two URIs. Today the task collection
is:

- `GET /all_tasks` — every task the caller can see,
- `GET /incomplete_tasks` — the same set, filtered to `completed = false`.

The two differ by one predicate, and both run the same query with one extra
`WHERE` clause (`get_all_tasks_for_user` at `core.rs:442-472`,
`get_incomplete_tasks_for_user` at `core.rs:474-505` — near-duplicates). Two
URIs for one collection means two handlers to keep correct, two documents to
maintain, and no way to express a filter that was not anticipated when the
endpoints were written.

The filter is not new to clients: `PATCH /task_by_id` already sets
`completed`, and `Task` already serializes it, so the query parameter matches
the representation.

## In scope

- `GET /tasks` returns everything the caller can see: tasks they own plus
  tasks shared with them. Replaces `/all_tasks`.
- `GET /tasks?completed=false` returns the filtered view. Replaces
  `/incomplete_tasks`.
  - Decide the parameter's shape and document it. Recommended:
    `?completed=false` for a boolean filter, with the parameter optional and
    absent meaning "no filter". Avoid a second parameter for "incomplete" —
    that reintroduces two ways to say the same thing.
  - Reject a non-boolean `completed` value with `400` and the standard JSON
    error body, via a `web::Query` struct plus the existing
    `query_error_handler` (`web_api.rs:244-249`).
- Collapse the two core functions into one that takes the filter, e.g.
  `get_tasks_for_user(user, completed: Option<bool>, connection)`, so the
  "owned or shared" subquery exists once. `get_all_tasks_for_user` and
  `get_incomplete_tasks_for_user` are removed rather than kept as wrappers.
- The collection keeps the current response envelope; ticket 014 removes it
  once, after every URI has moved.

## Out of scope

- Filtering by owner, share state, or title. Those are plausible next filters
  and the `web::Query` struct should make adding them cheap, but adding them
  now is scope creep.
- Pagination. The collections are unbounded today and that is a separate
  problem; note it as a follow-up rather than solving it here.
- Sorting. Keep the current database ordering.

## Acceptance criteria

- [ ] `GET /tasks` returns the caller's tasks, shared ones included.
- [ ] `GET /tasks?completed=false` returns only incomplete ones.
- [ ] `GET /tasks?completed=true` returns only completed ones.
- [ ] `GET /tasks?completed=maybe` returns `400` with the standard JSON error
      body.
- [ ] `GET /tasks` with no `completed` parameter behaves like the old
      `/all_tasks`.
- [ ] Only one core query function remains; the two old ones are gone.
- [ ] Tests updated: `can_get_all_tasks`, `can_get_incomplete_tasks`,
      `read_share_grants_get_and_listing_access` (which uses
      `/incomplete_tasks`), `read_write_share_grants_get_and_listing_access`,
      `owner_listings_never_duplicate_with_stale_share_row`,
      `no_share_means_401_and_no_listing_entry`.
- [ ] New test: the same filter returns the same task set whether it is
      reached with `?completed=false` or by default plus client-side filtering
      — or, more usefully, that a task completed by another user disappears
      from the caller's filtered view.
- [ ] `README.md` documents the collection, the filter, and the parameter's
      error behavior; `/all_tasks` and `/incomplete_tasks` are removed.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

The duplicated `owned OR shared` subquery in the two core functions is the
maintenance argument for this ticket beyond the naming: a fix to the sharing
rules has to be made twice today.

Do not add `?owner=me` style shortcuts. A caller who owns a task already sees
it in `GET /tasks`.
