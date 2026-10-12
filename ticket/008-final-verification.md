# Ticket 010: Final verification

**Depends on:** tickets 001–007
**Files:** none (verification only)

## Goal

Confirm the migration is complete and consistent across the whole codebase.
Per-ticket verification (fmt/clippy/test after every ticket) is already
enforced by tickets 001–007; this ticket is the final sweep.

## Steps

- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `grep` confirms no remaining `web::Json` usage in GET handlers and none
      of the old payload structs (`GetAllTasksPayload`,
      `GetIncompleteTasksPayload`, `GetTaskByIdPayload`, `GetUserByIdPayload`,
      `WhoAmIPayload`, `GetTaskSharesPayload`) exist.
- [ ] Every GET endpoint ticket's documentation updates are present in
      `README.md` and `http_requests/`.
- [ ] Manual smoke test (optional): run the server and exercise one migrated
      endpoint with `curl -H "Authorization: Bearer <key>" \
      "http://127.0.0.1:<port>/task_by_id?id=1"`.
