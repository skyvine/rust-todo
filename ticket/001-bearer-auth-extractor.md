# Ticket 001: Shared `Authorization: Bearer` extractor

**Depends on:** none
**Files:** `src/web_api.rs`, `src/lib.rs`

## Goal

Add the shared plumbing that every migrated GET endpoint will use:

1. An `AuthKey` newtype that zeroizes on drop (consistent with the existing
   `ZeroizeOnDrop` payload structs).
2. An extractor (`FromRequest`) for `AuthKey` that parses
   `Authorization: Bearer <auth_key>` and returns the key.
3. A `QueryConfig` error handler so that a missing/malformed query parameter
   produces the standard JSON 400 error body instead of actix's plain-text
   default.

## Changes

- In `src/web_api.rs`:
  - Add `AuthKey(String)` with `#[derive(ZeroizeOnDrop)]` and an accessor.
  - Implement `FromRequest` for `AuthKey` (or a `BearerAuth` wrapper struct):
    - Missing header, non-`Bearer` scheme, or empty key → `401` with
      `error_body(request_id)`; generate the `request_id` with `Uuid::new_v4()`
      inside the extractor.
  - Add a `query_error_handler` helper producing `400` with
    `error_body(request_id)`.
- In `src/lib.rs`:
  - Register the handler in `build_app!` via
    `app_data(web::QueryConfig::default().error_handler(query_error_handler))`.

## Acceptance criteria

- [ ] A request with `Authorization: Bearer <key>` yields the key in the handler.
- [ ] A missing `Authorization` header returns `401` with a JSON body
      containing `request_id`.
- [ ] A malformed header (wrong scheme, missing token) returns `401` with the
      same JSON error body.
- [ ] A request with a missing or non-numeric `?id=` returns `400` with a
      JSON body containing `request_id`.
- [ ] The auth key is zeroized on drop.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes (no endpoint behavior changed yet).
