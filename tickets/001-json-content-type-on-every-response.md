# 001: Send `Content-Type: application/json` on every JSON response

**Phase:** media types, headers, and status codes (before the redesign)
**Depends on:** none
**Files:** `src/web_api.rs`

## Goal

Every response whose body is JSON must declare `Content-Type:
application/json`, so a client never has to guess the media type — including
strict clients, proxies, and generated API documentation.

## Background

Every handler builds its response by hand:

```rust
HttpResponse::Ok().body(format!("{}", json!({"request_id": ..., "task": task})))
```

`HttpResponseBuilder::body` does not set a media type. In actix-web 4.11 only
`.json()` does (`actix-web-4.11.0/src/response/builder.rs:333-350`, which
inserts `mime::APPLICATION_JSON` when no `Content-Type` is already present).
Nothing in `src/web_api.rs` calls `.json(...)` or `.content_type(...)`, and
the extractor error paths (`unauthorized_error`,
`unauthorized_basic_error`, `query_error_handler`) build their JSON bodies the
same way.

Verified against a running server: every JSON response goes out with only
`content-length` and `date` — no media type at all.

```
HTTP/1.1 200 OK
content-length: 135
date: Sat, 17 Oct 2026 11:45:34 GMT

{"request_id":"da280185-…","task":{"completed":false,…}}
```

`README.md` line 24 claims "All data is sent and received as JSON", so this is
also a documentation/implementation mismatch.

## In scope

- Stop round-tripping JSON through `format!`/`String` and use
  `HttpResponse::…::json(value)`.
- Cover every body-bearing response:
  - success: `get_all_tasks`, `get_incomplete_tasks`, `get_task_by_id`,
    `get_task_shares`, `get_user_by_id`, `whoami`, `add_task`, `login`,
    `share_task`, `update_task_by_id`, `unshare_task`.
  - errors: `ApplicationError::into_http_response`,
    `error_body_with_message`, `unauthorized_error`,
    `unauthorized_basic_error`, `query_error_handler`, and the 400s from the
    `web::Json` / `web::Query` extractors.
- The `error_body` / `error_body_with_message` helpers should return a
  `serde_json::Value` so callers can hand them straight to `.json(...)`
  instead of re-serializing a string. (Their call sites in ticket 014 will
  change too; keep the signature change small and mechanical.)
- Empty-body responses (`HttpResponse::Created().finish()` in
  `register_account`, `HttpResponse::Ok().finish()` in `logout`) must **not**
  gain a `Content-Type` — there is no representation to describe.

## Out of scope

- Content negotiation (`Accept`, `application/ld+json`, and other
  representations) and `406 Not Acceptable`.
- Choosing a different media type or adding `Content-Type` to empty bodies.
- The `{"request_id": …}` envelope itself (ticket 014).

## Acceptance criteria

- [ ] Every JSON-bearing response, including all 4xx and 401 error paths,
      carries `Content-Type: application/json`.
- [ ] Empty-body responses carry no `Content-Type`.
- [ ] New test: a success response with a body, an error response (e.g. the 400
      from `/task_by_id` with a bad query string), and a 401 all assert the
      media type.
- [ ] No response body is built by `format!("{}", json!(…))` any more, unless
      it is passed to `.json(...)`.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

This is the cheapest fix in the audit, and it changes no URI, request, or
response shape, so it is sequenced before the redesign and nothing downstream
has to revisit it.
