use crate::core::{
    ApplicationError, Permission, Share, SharePermission, ShareWrite, add_share_for_task, add_user,
    auth_key_to_user, establish_connection, get_new_auth_key, get_task_permission,
    get_tasks_for_user, get_user_by_name, list_shares_for_task, remove_share, revoke_auth_key,
};
use crate::domain_types::{
    AuthKey, CleartextPassword, Credentials, TaskDescription, TaskTitle, Username,
};

use actix_web::{
    Error, FromRequest, HttpRequest, dev::Payload, error::InternalError, error::JsonPayloadError,
    error::QueryPayloadError,
};
use actix_web::{HttpResponse, Responder, delete, get, patch, post, put};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, SaltString, rand_core::OsRng},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::{Ready, ready};
use tracing::{Level, event, span};
use uuid::Uuid;
use zeroize::ZeroizeOnDrop;

/// The body shared by all error responses: the request id, plus an
/// optional message for client errors.
///
/// It is returned as a [`serde_json::Value`] so that every response built from
/// it can hand it straight to `HttpResponseBuilder::json`, which sets the
/// `Content-Type` describing the body.
fn error_body(request_id: impl std::fmt::Display) -> serde_json::Value {
    json!({"request_id": request_id.to_string()})
}

/// The body of a client error: the request id, plus a message explaining what
/// the client got wrong.
///
/// Like [`error_body`], it is returned as a [`serde_json::Value`] so that
/// responses can be built with `HttpResponseBuilder::json`, which sets the
/// `Content-Type` describing the body.
fn error_body_with_message(
    request_id: impl std::fmt::Display,
    message: impl Serialize,
) -> serde_json::Value {
    json!({"request_id": request_id.to_string(), "message": message})
}

impl ApplicationError {
    /// The status and body this error becomes, with no challenge attached.
    ///
    /// Every endpoint answers an error the same way; only the protection space
    /// the request was made in decides the challenge a 401 or a 403 carries.
    fn into_status_response(self, request_id: &String) -> HttpResponse {
        match self {
            ApplicationError::DieselError(e) => {
                event!(Level::ERROR, "Diesel error: {e}");
                match e {
                    diesel::result::Error::NotFound => {
                        HttpResponse::NotFound().json(error_body(request_id))
                    }
                    _ => HttpResponse::InternalServerError().json(error_body(request_id)),
                }
            }
            ApplicationError::InvalidAuthKey => {
                event!(Level::ERROR, "Invalid auth key");
                HttpResponse::Unauthorized().json(error_body(request_id))
            }
            ApplicationError::InvalidData(message) => {
                event!(Level::ERROR, "Invalid data: {message}");
                HttpResponse::BadRequest().json(error_body_with_message(request_id, message))
            }
            ApplicationError::InvalidPassword => {
                event!(Level::ERROR, "Invalid password");
                HttpResponse::Unauthorized().json(error_body(request_id))
            }
            ApplicationError::QueryFailed(e) => {
                event!(Level::ERROR, "Query failed: {e}");
                HttpResponse::InternalServerError().json(error_body(request_id))
            }
            ApplicationError::Forbidden => {
                event!(Level::ERROR, "Forbidden");
                HttpResponse::Forbidden().json(error_body(request_id))
            }
            ApplicationError::Unauthorized => {
                event!(Level::ERROR, "Unauthorized");
                HttpResponse::Unauthorized().json(error_body(request_id))
            }
            ApplicationError::UserExists => HttpResponse::Conflict().json(error_body(request_id)),
        }
    }

    /// The challenge which a 401 or a 403 from a Bearer-protected endpoint
    /// answers with: the bearer scheme, naming the realm, plus the RFC 6750
    /// §3.1 error code for the failure when there is one.
    ///
    /// Only a 401 and a 403 carry a challenge — [`with_challenge`] hands every
    /// other status back untouched — and RFC 9110 asks for nothing beyond the
    /// scheme and realm, so that is the whole answer by default.
    fn bearer_challenge(&self) -> &'static str {
        match self {
            // An auth key was presented and then refused, so RFC 6750 §3.1
            // calls this `invalid_token`: the client needs new credentials.
            ApplicationError::InvalidAuthKey => BEARER_INVALID_TOKEN_CHALLENGE,
            // The auth key was accepted and the request still may not be
            // fulfilled, so RFC 6750 §3.1 calls this `insufficient_scope`.
            ApplicationError::Forbidden => BEARER_INSUFFICIENT_SCOPE_CHALLENGE,
            _ => BEARER_CHALLENGE,
        }
    }

    /// The response for an error on a Bearer-protected endpoint: the status
    /// and body every endpoint shares, plus the bearer challenge on a 401 or
    /// a 403.
    fn into_http_response(self, request_id: &String) -> HttpResponse {
        let challenge = self.bearer_challenge();
        with_challenge(self.into_status_response(request_id), challenge)
    }

    /// The response for an error on the endpoint which takes credentials
    /// (`POST /auth_keys`): the same status and body, with the Basic challenge
    /// on a 401 instead.
    ///
    /// It is the only endpoint which takes credentials, so this is the only
    /// place the Basic scheme is offered: a 401 anywhere else answers with a
    /// bearer challenge.
    fn into_basic_http_response(self, request_id: &String) -> HttpResponse {
        with_basic_challenge(self.into_status_response(request_id))
    }
}

// Shared request extractors

/// The scheme an `Authorization` header must use to carry an auth key.
const BEARER_SCHEME: &str = "Bearer";

/// The scheme an `Authorization` header must use to carry a username and a
/// password.
const BASIC_SCHEME: &str = "Basic";

/// The challenge which every 401 from the endpoint which takes credentials
/// (`POST /auth_keys`) carries, naming the protection space this server guards.
///
/// RFC 7617 requires the realm and permits the optional charset, whose only
/// allowed value is UTF-8: that is the encoding this server reads credentials
/// as.
const BASIC_CHALLENGE: &str = "Basic realm=\"todo\", charset=\"UTF-8\"";

/// The challenge which a 401 or a 403 from a Bearer-protected endpoint falls
/// back to when no RFC 6750 error code applies, naming the same realm as
/// [`BASIC_CHALLENGE`]: the whole API is one protection space, only the scheme
/// which guards it differs.
const BEARER_CHALLENGE: &str = "Bearer realm=\"todo\"";

/// The challenge for a request whose `Authorization` header was missing or
/// malformed, so no auth key ever reached the server to be judged.
///
/// RFC 6750 §3.1 calls this `invalid_request`, and its advice is to fix the
/// request rather than to get new credentials.
const BEARER_INVALID_REQUEST_CHALLENGE: &str = "Bearer realm=\"todo\", error=\"invalid_request\"";

/// The challenge for a request which presented an auth key that was unknown or
/// expired.
///
/// RFC 6750 §3.1 calls this `invalid_token`, and its advice is to get new
/// credentials.
const BEARER_INVALID_TOKEN_CHALLENGE: &str = "Bearer realm=\"todo\", error=\"invalid_token\"";

/// The challenge for a request whose auth key was accepted but which asks for
/// more than that user may have: someone else's task, or a task the caller
/// may only read.
///
/// RFC 6750 §3.1 calls this `insufficient_scope`, and its advice is to ask for
/// more scope, not to authenticate again — retrying with another key would
/// fail the same way.
const BEARER_INSUFFICIENT_SCOPE_CHALLENGE: &str =
    "Bearer realm=\"todo\", error=\"insufficient_scope\"";

impl AuthKey {
    /// Reads the key out of an `Authorization` header.
    ///
    /// The header is exactly `Bearer <key>`: a missing header, another scheme,
    /// an empty key or a trailing word are all malformed.
    fn from_header(req: &HttpRequest, request_id: Uuid) -> Result<Self, Error> {
        let mut words = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .split_whitespace();

        let scheme = words.next();
        let key = words.next();
        let end = words.next();

        if scheme.is_some() && !scheme.unwrap().eq_ignore_ascii_case(BEARER_SCHEME) {
            event!(
                Level::ERROR,
                %request_id,
                "Incorrect scheme \"{:?}\"", scheme
            );
        }

        if end.is_some() {
            event!(Level::ERROR, %request_id, "Extra token after key: {:?}", end);
        }

        match (scheme, key, end) {
            // Schemes are case insensitive, so `bearer` is `Bearer` as well.
            (Some(scheme), Some(key), None) if scheme.eq_ignore_ascii_case(BEARER_SCHEME) => {
                Ok(AuthKey::new(String::from(key)))
            }
            _ => Err(unauthorized_error(request_id)),
        }
    }
}

fn unauthorized_error(request_id: Uuid) -> Error {
    // The header itself is deliberately not logged: it may hold a valid key.
    // The id is logged because the handler never runs, so no span carries it.
    event!(
        Level::ERROR,
        %request_id,
        "Missing or malformed Authorization header"
    );
    InternalError::from_response(
        "Missing or malformed Authorization header",
        with_challenge(
            HttpResponse::Unauthorized().json(error_body(request_id)),
            BEARER_INVALID_REQUEST_CHALLENGE,
        ),
    )
    .into()
}

/// Puts a challenge on a 401 or a 403, naming the scheme the client may retry
/// with.
///
/// RFC 9110 asks for a challenge on every 401 of a protected resource, whether
/// the request arrived without credentials or with credentials which were
/// refused. A 403 in a Bearer-protected endpoint carries one too: RFC 9110
/// §15.5.4 lets an origin server that wants to be clear that authentication
/// succeeded and the request was still refused say so in this header, and RFC
/// 6750 §3 supplies `insufficient_scope` for exactly that case. Any other
/// status is handed back untouched, so the challenge never leaks onto a 400 or
/// a 500.
fn with_challenge(mut response: HttpResponse, challenge: &'static str) -> HttpResponse {
    if matches!(
        response.status(),
        actix_web::http::StatusCode::UNAUTHORIZED | actix_web::http::StatusCode::FORBIDDEN
    ) {
        response.headers_mut().insert(
            actix_web::http::header::WWW_AUTHENTICATE,
            actix_web::http::header::HeaderValue::from_static(challenge),
        );
    }
    response
}

/// Puts the Basic challenge on a 401 from the endpoint which takes
/// credentials.
///
/// That endpoint is the only one which takes credentials, so it is the only
/// place the Basic scheme is ever offered: every other challenge names its own
/// scheme at its call site instead of relying on this one replacing it.
fn with_basic_challenge(response: HttpResponse) -> HttpResponse {
    with_challenge(response, BASIC_CHALLENGE)
}

/// Turns a missing or malformed Basic `Authorization` header into the 401 that
/// RFC 7617 calls for: the same JSON body as every other failure, plus the
/// challenge which tells the client which scheme to retry with.
fn unauthorized_basic_error(request_id: Uuid) -> Error {
    // The header itself is deliberately not logged: it carries the password.
    // The id is logged because the handler never runs, so no span carries it.
    event!(
        Level::ERROR,
        %request_id,
        "Missing or malformed Basic Authorization header"
    );
    InternalError::from_response(
        "Missing or malformed Basic Authorization header",
        with_basic_challenge(HttpResponse::Unauthorized().json(error_body(request_id))),
    )
    .into()
}

impl FromRequest for AuthKey {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        // The handler never runs, so the id of the rejected request has to be
        // generated here rather than alongside the handler's own span.
        let request_id = Uuid::new_v4();
        ready(AuthKey::from_header(req, request_id))
    }
}

impl Credentials {
    /// Reads a username and a password out of an `Authorization` header.
    ///
    /// The header is exactly `Basic <base64(username:password)>`: a missing
    /// header, another scheme, a token which is not base64 or valid UTF-8, a
    /// token without a colon and a trailing word are all malformed.
    fn from_header(req: &HttpRequest, request_id: Uuid) -> Result<Self, Error> {
        let mut words = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .split_whitespace();

        let scheme = words.next();
        let encoded = words.next();
        let end = words.next();

        if scheme.is_some() && !scheme.unwrap().eq_ignore_ascii_case(BASIC_SCHEME) {
            event!(
                Level::ERROR,
                %request_id,
                "Incorrect scheme \"{:?}\"", scheme
            );
        }

        if end.is_some() {
            event!(
                Level::ERROR,
                %request_id,
                "Extra token after credentials: {:?}", end
            );
        }

        match (scheme, encoded, end) {
            // Schemes are case insensitive, so `basic` is `Basic` as well.
            (Some(scheme), Some(encoded), None) if scheme.eq_ignore_ascii_case(BASIC_SCHEME) => {
                let decoded = match STANDARD.decode(encoded) {
                    Ok(decoded) => decoded,
                    Err(_) => return Err(unauthorized_basic_error(request_id)),
                };
                let decoded = match String::from_utf8(decoded) {
                    Ok(decoded) => decoded,
                    Err(_) => return Err(unauthorized_basic_error(request_id)),
                };
                // Only the first colon separates the two halves, so a password
                // is free to contain colons of its own.
                match decoded.split_once(':') {
                    Some((username, password)) => Ok(Credentials::new(
                        String::from(username),
                        String::from(password),
                    )),
                    None => Err(unauthorized_basic_error(request_id)),
                }
            }
            _ => Err(unauthorized_basic_error(request_id)),
        }
    }
}

impl FromRequest for Credentials {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let request_id = Uuid::new_v4();
        ready(Credentials::from_header(req, request_id))
    }
}

/// Turns a query string actix refuses to deserialize into the same JSON error
/// body as the rest of the API, instead of actix's plain text default.
///
/// Registered app-wide in `build_app!` as the `QueryConfig` error handler.
pub fn query_error_handler(err: QueryPayloadError, _req: &HttpRequest) -> Error {
    let request_id = Uuid::new_v4();
    // The id is logged because the handler never runs, so no span carries it.
    event!(
        Level::ERROR,
        %request_id,
        "Unable to read the query string: {err}"
    );
    InternalError::from_response(err, HttpResponse::BadRequest().json(error_body(request_id)))
        .into()
}

/// Turns a request body actix refuses to deserialize as JSON into the same
/// JSON error body as the rest of the API, instead of an empty body.
///
/// Registered app-wide in `build_app!` as the `JsonConfig` error handler.
pub fn json_error_handler(err: JsonPayloadError, _req: &HttpRequest) -> Error {
    let request_id = Uuid::new_v4();
    event!(
        Level::ERROR,
        %request_id,
        "Unable to read the request body as JSON: {err}"
    );
    InternalError::from_response(err, HttpResponse::BadRequest().json(error_body(request_id)))
        .into()
}

/// The path parameter every task URI carries: the id of the task it names.
///
/// The id lives in the URI rather than in a query string or a body field, so
/// the path is what identifies the task. A segment which cannot be an id names
/// no task at all, and the router answers that with a 404 rather than an
/// extractor answering 400 for a request which named nothing.
#[derive(Deserialize)]
struct TaskPath {
    id: i32,
}

/// The path parameters every single-share URI carries: the id of the task the
/// share belongs to, and the username of the user it shares with.
///
/// Both halves live in the URI rather than in a body, so the URI alone names
/// the share: a client which has seen a share listed can change it or remove
/// it without repeating anything else, and a `DELETE` needs nothing in a body
/// to say which share to remove.
#[derive(Deserialize)]
struct SharePath {
    id: i32,
    username: String,
}

/// The URI of one share: where it lives, and where it is read, changed and
/// removed from.
///
/// A created share answers with this in a `Location`, so it is built in one
/// place rather than spelled out where it is needed.
fn share_uri(task_id: i32, username: &str) -> String {
    format!("/tasks/{task_id}/shares/{username}")
}

/// The URI of one auth key: the one URI which identifies it.
///
/// The segment is the key row's own id, never the key value. An auth key is a
/// bearer token, and RFC 6750 §5.3 keeps those out of URLs: every proxy and log
/// which records a request line would record the credential, and no response
/// header governs that. The id identifies the row without being secret, which
/// is how every other resource here is addressed too — a task by its id, a share
/// by its username.
fn auth_key_uri(id: i32) -> String {
    format!("/auth_keys/{id}")
}

/// A share in a response body: the id of the share itself, the username it is
/// for, and the permission it grants.
///
/// The listing and the answer to a `PUT` both build their entries here, so one
/// share is never represented two ways.
fn share_body(share: &Share) -> serde_json::Value {
    json!({
        "id": share.id(),
        "username": share.username(),
        "permission": share.permission().as_stored_text(),
    })
}

// Auth keys

/// Creates an auth key for the account the credentials name, and answers
/// `201 Created` with a `Location` naming the key's URI.
///
/// The credentials are read from an HTTP Basic `Authorization` header, so a
/// request has no body. Every 401 answer, whether the header was missing or
/// malformed or the credentials were refused, carries the Basic challenge that
/// `WWW-Authenticate` requires.
///
/// A refused attempt is refused the same way whether or not the username
/// exists: an unknown username is a refused authentication attempt, not a
/// missing resource, so it answers 401 and never 404.
///
/// The two halves of the answer are deliberately different things. The `Location`
/// names the new key by its id, so the URI identifies it without being a
/// credential; the body carries the key itself, which is the secret the client
/// needs. A client therefore never has to ask a second question to learn what
/// it was handed, and no request to this API ever puts a bearer token in a URI.
///
/// The API stays stateless: the key is presented in `Authorization: Bearer` on
/// every later request, and this endpoint is the only one which takes
/// credentials rather than a key.
#[post("/auth_keys")]
pub async fn create_auth_key(credentials: Credentials) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Create Auth Key", %request_id).entered();

    let (raw_username, pw) = credentials.into_parts();

    let un = match Username::new(raw_username) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().json(error_body_with_message(request_id, message));
        }
    };

    match establish_connection() {
        Ok(mut conn) => {
            let user = match get_user_by_name(&un, &mut conn) {
                Ok(user) => user,
                Err(e) => {
                    return match e {
                        // A username nobody has registered is a refused
                        // authentication attempt, not a missing resource:
                        // neither this endpoint nor the protection space it
                        // guards is absent, so RFC 9110 §15.5.2 describes the
                        // answer and §15.5.5 does not. It is built by hand
                        // rather than routed through the error mapper so that
                        // a lookup miss here no longer decides the status a
                        // client sees, and so the refusal is logged at the
                        // level the rest of this endpoint logs at.
                        ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                            event!(Level::TRACE, "No account named {}", un.as_ref());
                            with_basic_challenge(
                                HttpResponse::Unauthorized().json(error_body(request_id)),
                            )
                        }
                        other => other.into_basic_http_response(&format!("{request_id}")),
                    };
                }
            };

            let hashed_password = match PasswordHash::new(user.ref_password()) {
                Ok(ph) => ph,
                Err(e) => {
                    event!(
                        Level::ERROR,
                        "Unable to parse hashed password ({}): {e:?}",
                        user.ref_password()
                    );
                    return HttpResponse::InternalServerError().json(error_body(request_id));
                }
            };

            match get_new_auth_key(&user, &pw, &hashed_password, &mut conn) {
                Ok(created) => HttpResponse::Created()
                    .insert_header((
                        actix_web::http::header::LOCATION,
                        auth_key_uri(created.id()),
                    ))
                    .json(
                        json!({ "request_id": format!("{request_id}"), "auth_key": created.key() }),
                    ),
                Err(e) => e.into_basic_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

/// Revokes the auth key the caller presented, so it stops working at once.
///
/// The key is the one in the `Authorization` header rather than one named by
/// the URI: the API never hands out a key other than the one the caller
/// presented, so there is no way to name somebody else's key, and a caller
/// revoking its own key is all this does. Naming a session instead would be a
/// different resource, and this API has none.
///
/// A key which is not there is not a failure: `DELETE` is idempotent, so
/// revoking a key which was already revoked leaves the state the caller asked
/// for and is answered exactly as the removal which got it there was.
#[delete("/auth_keys")]
pub async fn delete_auth_key(auth_key: AuthKey) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Delete Auth Key", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => match revoke_auth_key(auth_key.as_str(), &mut connection) {
            Ok(()) => HttpResponse::NoContent().finish(),
            Err(e) => e.into_http_response(&format!("{request_id}")),
        },
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

// GET endpoints

/// The filter `GET /tasks` reads from the query string.
///
/// `completed` is optional, and leaving it out means no filter: the whole
/// collection the caller can see. It is a boolean rather than a second
/// parameter for "incomplete" because it is the field the same name carries in
/// the task representation, and a client which has one knows which value to
/// send. A value which is not a boolean names no view of the collection, so it
/// is a client error: `query_error_handler` answers 400 with the standard JSON
/// error body.
#[derive(Deserialize)]
struct TasksQuery {
    completed: Option<bool>,
}

/// Returns the caller's collection of tasks: the ones they own plus the ones
/// shared with them, narrowed to `completed` when that filter is given.
///
/// The filter narrows the collection rather than naming a second one: the same
/// URI serves both, and the task's completion state is a property of the task,
/// not of the way it is asked for.
#[get("/tasks")]
pub async fn get_tasks(
    auth_key: AuthKey,
    query: actix_web::web::Query<TasksQuery>,
) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Tasks", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let user = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match get_tasks_for_user(&user, query.completed, &mut connection) {
                Ok(tasks) => HttpResponse::Ok()
                    .json(json!({"request_id": format!("{request_id}"), "tasks": tasks})),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

/// Returns the task named by the URI.
///
/// The user must be logged in as the task's owner or have a share on
/// the task in order to get the task.
#[get("/tasks/{id}")]
pub async fn get_task_by_id(
    auth_key: AuthKey,
    path: actix_web::web::Path<TaskPath>,
) -> impl Responder {
    let task_id = path.into_inner().id;
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Task by ID", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            // Authenticate before fetching anything
            let user = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            // Owner, read-only, and read-write share holders may all view
            // the task; anyone else gets the same 403.
            match get_task_permission(&user, &task_id, &mut connection) {
                Ok(Permission::Owner) | Ok(Permission::ReadOnly) | Ok(Permission::ReadWrite) => {
                    match crate::core::get_task_by_id(&task_id, &mut connection) {
                        Ok(task) => HttpResponse::Ok()
                            .json(json!({"request_id": format!("{request_id}"), "task": task})),
                        Err(e) => e.into_http_response(&format!("{request_id}")),
                    }
                }
                Ok(_) => {
                    event!(
                        Level::ERROR,
                        "Cannot get task {}: user {} has no task access",
                        task_id,
                        user.ref_id()
                    );
                    ApplicationError::Forbidden.into_http_response(&format!("{request_id}"))
                }
                Err(e) => match e {
                    // Don't distinguish "task doesn't exist" from "not your
                    // task": both are the same 403 to avoid leaking which
                    // task IDs exist.
                    ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                        event!(Level::ERROR, "Cannot get task {}: not found", task_id);
                        ApplicationError::Forbidden.into_http_response(&format!("{request_id}"))
                    }
                    other => other.into_http_response(&format!("{request_id}")),
                },
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

/// An endpoint to checks that the server is up.
///
/// This endpoint always responds with status OK simply to verify that
/// the server is running and responding to requests.
#[get("/is_alive")]
pub async fn is_alive() -> impl Responder {
    event!(Level::TRACE, "Responding to is_alive check");
    HttpResponse::Ok()
}

/// Lists the shares on a task owned by the caller.
///
/// The shares are a sub-resource of the task, so they live under its URI and
/// the task is named by a path segment rather than by a query parameter. Only
/// the owner may list them, and a task which does not exist is answered
/// exactly as a task owned by someone else is, so the listing cannot be used
/// to find out which task ids exist.
#[get("/tasks/{id}/shares")]
pub async fn get_task_shares(
    auth_key: AuthKey,
    path: actix_web::web::Path<TaskPath>,
) -> impl Responder {
    let task_id = path.into_inner().id;
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Task Shares", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match list_shares_for_task(&owner, &task_id, &mut connection) {
                Ok(shares) => {
                    let shares: Vec<serde_json::Value> = shares.iter().map(share_body).collect();
                    HttpResponse::Ok()
                        .json(json!({"request_id": format!("{request_id}"), "shares": shares}))
                }
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

/// The query `GET /user_by_id` reads the user id from.
#[derive(Deserialize)]
struct UserByIdQuery {
    id: i32,
}

/// Resolves a user id (as exposed in `Task`'s `owner` field and in share
/// responses) to that user's username. User ids are public, so an unknown
/// id simply returns 404.
#[get("/user_by_id")]
pub async fn get_user_by_id(
    auth_key: AuthKey,
    query: actix_web::web::Query<UserByIdQuery>,
) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get User By ID", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let _user = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::get_user_by_id(&query.id, &mut connection) {
                Ok(user) => HttpResponse::Ok().json(
                    json!({"request_id": format!("{request_id}"), "username": user.ref_username()}),
                ),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

/// Returns the username of the currently logged in user.
#[get("/whoami")]
pub async fn whoami(auth_key: AuthKey) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Who Am I Request", %request_id).entered();

    match establish_connection() {
        Ok(mut conn) => match auth_key_to_user(auth_key.as_str(), &mut conn) {
            Ok(user) => HttpResponse::Ok().json(
                json!({ "request_id": format!("{}", request_id), "username": user.ref_username()}),
            ),
            Err(e) => {
                event!(Level::ERROR, "Unable to look up user");
                e.into_http_response(&format!("{request_id}"))
            }
        },
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

// POST endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct AddTaskPayload {
    title: String,
    description: Option<String>,
}

/// Creates a task in the collection and answers with its representation.
///
/// "Completed" defaults to "false". The `Location` header names the URI of the
/// new task, which is where it can be read and changed from then on.
#[post("/tasks")]
pub async fn add_task(
    auth_key: AuthKey,
    mut payload: actix_web::web::Json<AddTaskPayload>,
) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Add Task", %request_id).entered();

    let title = match TaskTitle::new(std::mem::take(&mut payload.title)) {
        Ok(title) => title,
        Err(e) => {
            event!(Level::ERROR, "{e:?}");
            return HttpResponse::BadRequest().json(error_body_with_message(request_id, e));
        }
    };
    let description = payload.description.take().map(TaskDescription::new);
    let description = description.as_ref();

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::add_task(&owner, &title, false, description, &mut connection) {
                Ok(task) => HttpResponse::Created()
                    .insert_header((
                        actix_web::http::header::LOCATION,
                        format!("/tasks/{}", task.id()),
                    ))
                    .json(json!({"request_id": format!("{request_id}"), "task": task})),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}
#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct UserRegistrationPayload {
    username: String,
    password: String,
}

/// Create an account with the given account info.
///
/// On a success, the user will be added to the database as a valid user and return HTTP OK.
///
/// In any error state this will create a log event at the ERROR level and return an appropriate
/// error HTTP response to send back to the client.
#[post("/register_account")]
pub async fn register_account(
    mut account_info: actix_web::web::Json<UserRegistrationPayload>,
) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Registering Account", %request_id).entered();

    // move not allowed in .values() call below, avoid cloning by replacing
    let un = match Username::new(std::mem::take(&mut account_info.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().json(error_body(request_id));
        }
    };
    let pw = CleartextPassword::new(std::mem::take(&mut account_info.password));

    let slt = SaltString::generate(&mut OsRng);

    // The default paramaters from Argon2 match one of the recommendations from
    // OWASP (see https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#argon2id)
    let hashed_password = match Argon2::default().hash_password(pw.as_ref().as_bytes(), &slt) {
        Ok(h) => h.to_string(),
        Err(e) => {
            event!(Level::ERROR, "Unable to hash password: {e:?}");
            return HttpResponse::InternalServerError().json(error_body(request_id));
        }
    };

    match establish_connection() {
        Ok(mut conn) => {
            event!(Level::TRACE, "Established connection to database.");

            match add_user(&un, &hashed_password, &mut conn) {
                Ok(()) => HttpResponse::Created().finish(),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }

        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e}."
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

// PATCH endpoints

/// The body of a `PATCH` on a task: only the fields to change.
///
/// The task the fields belong to is named by the URI, so the body carries no
/// identifier of its own.
#[derive(Default, Deserialize, PartialEq, Serialize)]
struct UpdateTaskPayload {
    completed: Option<bool>,
    title: Option<String>,
    description: Option<String>,
}

/// Changes one or more of the task's fields.
///
/// It is an error for all of the optional values to be None; at least on field must be updated for
/// this endpoint to succeed.
#[patch("/tasks/{id}")]
pub async fn update_task_by_id(
    auth_key: AuthKey,
    path: actix_web::web::Path<TaskPath>,
    payload: actix_web::web::Json<UpdateTaskPayload>,
) -> impl Responder {
    let task_id = path.into_inner().id;
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Update Task", %request_id).entered();
    let mut payload = payload.into_inner();

    // Reject no-op updates without enumerating individual fields: if no
    // optional field is set, this equals the default (all None).
    if payload == UpdateTaskPayload::default() {
        event!(Level::ERROR, "No fields provided to update");
        return HttpResponse::BadRequest().json(error_body_with_message(
            request_id,
            "At least one field must be provided",
        ));
    }

    let title = if let Some(title) = payload.title.as_mut() {
        match TaskTitle::new(std::mem::take(title)) {
            Ok(title) => Some(title),
            Err(e) => {
                event!(Level::ERROR, "{e:?}");
                return HttpResponse::BadRequest().json(error_body_with_message(request_id, e));
            }
        }
    } else {
        None
    };

    let description = payload
        .description
        .as_mut()
        .map(|d| TaskDescription::new(std::mem::take(d)));

    match establish_connection() {
        Ok(mut connection) => {
            let caller = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::update_task(
                &caller,
                &task_id,
                payload.completed,
                title,
                description,
                &mut connection,
            ) {
                Ok(()) => HttpResponse::Ok().json(error_body(request_id)),
                Err(e) => match e {
                    // Don't distinguish "task doesn't exist" from "not your
                    // task": both are the same 403 to avoid leaking which
                    // task IDs exist.
                    ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                        event!(Level::ERROR, "Cannot update task {}: not found", task_id);
                        ApplicationError::Forbidden.into_http_response(&format!("{request_id}"))
                    }
                    other => other.into_http_response(&format!("{request_id}")),
                },
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

// PUT endpoints

/// The body of a `PUT` on a share: the whole representation.
///
/// The share is named by the URI, so the body carries no identifier of its
/// own — the task, the sharee and the URI of the share are all in the path,
/// and the permission is the one field a share has left to say.
#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct SharePayload {
    permission: String,
}

/// Gives another user access to a task owned by the caller, at the share's own
/// URI, or replaces the permission on a share which is already there.
///
/// The URI says which share, and the body says what it grants, so this
/// assigns a permission to a fully-specified resource and replaces whatever
/// was there. That is `PUT` and not `POST`: the request is idempotent, and
/// repeating it leaves the share in the same state. The status is what tells
/// the two cases apart, since a client which cannot tell them cannot tell a
/// new share from a changed one — `201 Created` with a `Location` naming the
/// share's URI when it was created, `200 OK` when a permission was replaced.
/// Both answer with the share itself, so a client never has to follow the
/// `Location` to learn what it asked for.
#[put("/tasks/{id}/shares/{username}")]
pub async fn put_share(
    auth_key: AuthKey,
    path: actix_web::web::Path<SharePath>,
    payload: actix_web::web::Json<SharePayload>,
) -> impl Responder {
    let path = path.into_inner();
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Put Share", %request_id).entered();

    let target = match Username::new(path.username) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().json(error_body_with_message(request_id, message));
        }
    };

    let permission = match SharePermission::from_stored_text(&payload.permission) {
        Ok(p) => p,
        Err(e) => {
            event!(Level::ERROR, "Invalid permission: {e:?}");
            return HttpResponse::BadRequest().json(error_body_with_message(
                request_id,
                "permission must be \"read\" or \"read_write\"",
            ));
        }
    };

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match add_share_for_task(&owner, &path.id, &target, permission, &mut connection) {
                Ok(write) => {
                    let body = json!({"request_id": format!("{request_id}"), "share": share_body(write.share())});
                    match write {
                        ShareWrite::Created(_) => HttpResponse::Created()
                            .insert_header((
                                actix_web::http::header::LOCATION,
                                share_uri(path.id, write.share().username()),
                            ))
                            .json(body),
                        ShareWrite::Replaced(_) => HttpResponse::Ok().json(body),
                    }
                }
                // Forbidden covers both "task doesn't exist" and "not your
                // task", so the 403 doesn't reveal which.
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

// DELETE endpoints

/// Removes another user's access to a task owned by the caller, at the share's
/// own URI.
///
/// The request carries no body at all: the URI names the task and the sharee,
/// so there is nothing left to say in one. RFC 9110 §9.3.5 gives content in a
/// `DELETE` no defined meaning, and any intermediary which strips it would
/// otherwise break the request.
///
/// A share which is not there answers `204 No Content`, the same as one which
/// was. That is what makes repeating this request safe: the state the client
/// asked for is the state it already has. Only the owner may remove a share,
/// and a task which does not exist is answered exactly as a task owned by
/// someone else is.
#[delete("/tasks/{id}/shares/{username}")]
pub async fn delete_share(
    auth_key: AuthKey,
    path: actix_web::web::Path<SharePath>,
) -> impl Responder {
    let path = path.into_inner();
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Delete Share", %request_id).entered();

    let target = match Username::new(path.username) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().json(error_body_with_message(request_id, message));
        }
    };

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match remove_share(&owner, &path.id, &target, &mut connection) {
                Ok(()) => HttpResponse::NoContent().finish(),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().json(error_body(request_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{build_app, core::Task};
    use actix_http::Request;
    use actix_web::http::Method;
    use actix_web::http::header::{AUTHORIZATION, CONTENT_TYPE, LOCATION, WWW_AUTHENTICATE};
    use actix_web::{App, body::MessageBody, dev::Service, dev::ServiceResponse, test, web};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ctor::ctor;
    use tracing_subscriber::{EnvFilter, fmt};
    use uuid::Uuid;

    #[ctor]
    unsafe fn global_init() {
        use crate::schema::auth_keys::dsl::*;
        use crate::schema::tasks::dsl::*;
        use crate::schema::users::dsl::*;
        use diesel::prelude::*;

        // Print log messages to help debug failed tests
        fmt()
            .event_format(fmt::format().pretty())
            .with_env_filter(EnvFilter::from_default_env())
            .init();

        tracing::event!(tracing::Level::TRACE, "Deleting all entries in tables.");

        // The tests depend on the state of the database, so ensure we always start with a clean slate.
        match diesel::delete(auth_keys).execute(
            &mut super::establish_connection().expect("Unable to connect to test database."),
        ) {
            Ok(_) => (),
            Err(e) => panic!("Unable to delete auth_keys entries: {e:?}"),
        };

        match diesel::delete(tasks).execute(
            &mut super::establish_connection().expect("Unable to connect to test database."),
        ) {
            Ok(_) => (),
            Err(e) => panic!("Unable to delete tasks entries: {e:?}"),
        };

        match diesel::delete(users).execute(
            &mut super::establish_connection().expect("Unable to connect to test database."),
        ) {
            Ok(_) => (),
            Err(e) => panic!("Unable to delete users entries: {e:?}"),
        };
    }

    fn assert_response_success(response: ServiceResponse, message: &str) -> ServiceResponse {
        if !response.status().is_success() {
            let formatted_response = format!("{response:?}");
            // there is currently no "as_body" method or similar, so I have to take ownership and return if I want to print the body.
            let body = response.into_body();
            panic!("{message}: {formatted_response}{body:?}")
        }
        response
    }

    fn assert_client_error(response: ServiceResponse, message: &str) {
        if !response.status().is_client_error() {
            let formatted_response = format!("{response:?}");
            let body = response.into_body();
            panic!("{message}: {formatted_response}{body:?}")
        }
    }

    /// Asserts that a response carries exactly one challenge, the one named,
    /// and no other.
    ///
    /// Only one challenge may be sent per header field, so a client which
    /// found a second one would have no way to tell which scheme to retry
    /// with: the Basic challenge must not reach an endpoint which takes an
    /// auth key, nor the bearer challenge reach the one which takes
    /// credentials.
    fn assert_only_challenge(response: &ServiceResponse, expected: &str, message: &str) {
        let challenges: Vec<&str> = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .map(|value| {
                value
                    .to_str()
                    .expect("a challenge should be ASCII, as every scheme is")
            })
            .collect();
        assert_eq!(challenges, vec![expected], "{message}");
    }

    /// Asserts that a 401 carries the challenge a Basic client needs in order
    /// to know what to retry with.
    fn assert_basic_challenge(response: &ServiceResponse, message: &str) {
        assert_only_challenge(response, super::BASIC_CHALLENGE, message);
    }

    /// Asserts that a 403 names the reason the credentials were accepted and
    /// the request still refused, rather than asking for a new auth key.
    fn assert_insufficient_scope_challenge(response: &ServiceResponse, message: &str) {
        assert_only_challenge(
            response,
            super::BEARER_INSUFFICIENT_SCOPE_CHALLENGE,
            message,
        );
    }

    /// Asserts that a 403 says nothing should be retried with other
    /// credentials, since another auth key would be refused the same way.
    fn assert_forbidden(response: &ServiceResponse, message: &str) {
        assert_eq!(response.status().as_u16(), 403, "{message}");
        assert_insufficient_scope_challenge(response, message);
    }

    /// Asserts that a 401 asks the client to authenticate, rather than
    /// pretending its credentials were good but insufficient.
    fn assert_unauthorized(response: &ServiceResponse, message: &str) {
        assert_eq!(response.status().as_u16(), 401, "{message}");
    }

    /// Asserts the response carries the API's standard error body: a JSON
    /// object holding the id of the request that failed.
    fn assert_json_error_with_request_id(response: ServiceResponse) {
        let request_id = extract_json_string(response, "request_id");
        assert!(
            Uuid::parse_str(&request_id).is_ok(),
            "{request_id:?} is not a request id."
        );
    }

    /// Asserts that a response describes its body as JSON, so a client never
    /// has to guess the media type.
    fn assert_json_content_type(response: &ServiceResponse, message: &str) {
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json"),
            "{message}"
        );
    }

    /// Registers an account and returns the auth key it gets, the way a client
    /// which wants to go straight on to using the API does.
    async fn register_and_create_auth_key<E: std::fmt::Debug>(
        app: impl Service<Request, Response = ServiceResponse, Error = E>,
        username: &str,
        password: &str,
    ) -> String {
        assert_response_success(
            register(&app, username, password).await,
            "Unable to register account",
        );
        let created = assert_response_success(
            create_auth_key(&app, username, password).await,
            "Could not create an auth key.",
        );
        extract_json_string(created, "auth_key")
    }

    async fn register<E: std::fmt::Debug>(
        app: impl Service<Request, Response = ServiceResponse, Error = E>,
        username: &str,
        password: &str,
    ) -> ServiceResponse {
        let request = test::TestRequest::post()
            .uri("/register_account")
            .set_json(super::UserRegistrationPayload {
                username: String::from(username),
                password: String::from(password),
            })
            .to_request();
        test::call_service(&app, request).await
    }

    /// Exchanges credentials for an auth key at the auth key resource's own
    /// collection URI, the way a client authenticating for the first time does.
    async fn create_auth_key<E: std::fmt::Debug>(
        app: impl Service<Request, Response = ServiceResponse, Error = E>,
        username: &str,
        password: &str,
    ) -> ServiceResponse {
        let credentials = STANDARD.encode(format!("{username}:{password}"));
        let request = test::TestRequest::post()
            .uri("/auth_keys")
            .insert_header((AUTHORIZATION, format!("Basic {credentials}")))
            .to_request();
        test::call_service(&app, request).await
    }

    async fn add_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        title: &str,
        description: Option<&str>,
    ) -> ServiceResponse {
        let request = test::TestRequest::post()
            .uri("/tasks")
            .set_json(super::AddTaskPayload {
                title: String::from(title),
                description: description.map(String::from),
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    /// Creates a task and returns its id, which the created representation and
    /// the `Location` header both name.
    async fn create_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        title: &str,
        description: Option<&str>,
    ) -> i32 {
        let response = assert_response_success(
            add_task(app, auth_key, title, description).await,
            "Unable to add task.",
        );
        extract_json_from_constructor(response, "task", Task::from_json_object)
            .map(|task| *task.id())
            .expect("The created task should be in the response body.")
    }

    /// Lists the caller's tasks, narrowed by the `completed` filter when the
    /// caller passes one (`"?completed=false"`, say) and left unfiltered when
    /// the string is empty.
    async fn get_tasks(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        filter: &str,
    ) -> ServiceResponse {
        let request = test::TestRequest::get()
            .uri(&format!("/tasks{filter}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    fn extract_json_from_constructor<Constructor, Output>(
        response: ServiceResponse,
        key: &str,
        constructor: Constructor,
    ) -> Output
    where
        Constructor: FnOnce(&serde_json::Value) -> Output,
    {
        match response.into_body().try_into_bytes() {
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(bytes.as_ref()) {
                Ok(dict) => constructor(&dict[key]),
                Err(_) => panic!("Unable to deserialize alleged JSON: {bytes:?}"),
            },
            Err(e) => {
                panic!("Unable to extract bytes from response {e:?}")
            }
        }
    }

    fn extract_shares(response: ServiceResponse) -> Vec<serde_json::Value> {
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().try_into_bytes().unwrap()).unwrap();
        body["shares"]
            .as_array()
            .expect("shares should be an array")
            .clone()
    }

    fn extract_json_tasks(response: ServiceResponse) -> Vec<Task> {
        extract_json_from_constructor(response, "tasks", |v| match v.as_array() {
            Some(a) => a
                .iter()
                .map(|obj| match Task::from_json_object(obj) {
                    Ok(task) => task,
                    Err(e) => panic!("Could not parse task: {e:?}"),
                })
                .collect(),
            None => panic!("{} is not an array!", v),
        })
    }

    fn extract_json_string(response: ServiceResponse, key: &str) -> String {
        extract_json_from_constructor(response, key, |v| match v.as_str() {
            Some(s) => String::from(s),
            None => panic!("{} is not a string! {:?}", key, v),
        })
    }

    /// The keys of a response's JSON object, so two error bodies can be
    /// compared for shape.
    ///
    /// The `request_id` differs from request to request by design, so comparing
    /// bodies verbatim would compare nothing useful: what must match is the
    /// set of keys, which is what tells a client how to read the failure.
    fn json_body_keys(response: ServiceResponse) -> Vec<String> {
        let bytes = response
            .into_body()
            .try_into_bytes()
            .expect("Unable to extract bytes from response");
        let body: serde_json::Value = serde_json::from_slice(bytes.as_ref())
            .unwrap_or_else(|_| panic!("Unable to deserialize alleged JSON: {bytes:?}"));
        match body.as_object() {
            Some(object) => object.keys().cloned().collect(),
            None => panic!("{body} is not a JSON object!"),
        }
    }

    #[actix_web::test]
    async fn is_alive() {
        let app = test::init_service(build_app!()).await;
        let request = test::TestRequest::get().uri("/is_alive").to_request();
        let response = assert_response_success(
            test::call_service(&app, request).await,
            "Is alive check failed.",
        );
        assert_eq!(
            response.into_body().size(),
            actix_web::body::BodySize::Sized(0)
        );
    }

    #[actix_web::test]
    async fn regitering_account_is_successful() {
        let app = test::init_service(build_app!()).await;
        let response = register(
            app,
            "register-account-is-successful-name",
            "register-account-is-successful-password",
        )
        .await;
        assert_response_success(response, "Error status code");
    }

    #[actix_web::test]
    async fn duplicate_username_fails() {
        let name = String::from("duplicated-name");
        let password = String::from("duplicated-password");
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, name.as_str(), password.as_str()).await,
            "Failed to register the account once.",
        );

        let response = register(&app, name.as_str(), password.as_str()).await;
        assert_client_error(response, "Response did not indicate client failure");
    }

    #[actix_web::test]
    async fn auth_key_is_recognized() {
        let username = String::from("auth-key-registered-name");
        let password = String::from("auth-key-registered-password");
        let app = test::init_service(build_app!()).await;

        let auth_key =
            register_and_create_auth_key(&app, username.as_str(), password.as_str()).await;

        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let whoami_response = assert_response_success(
            test::call_service(&app, whoami_request).await,
            "Whoami request failed.",
        );

        let response_name = extract_json_string(whoami_response, "username");
        assert_eq!(username, response_name);
    }

    #[actix_web::test]
    async fn invalid_password_fails() {
        let username = "invalid-password-username";
        let password = "invalid-password-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, username, password).await,
            "Unable to register account",
        );

        let response = create_auth_key(&app, username, "not-the-correct-password").await;
        assert_client_error(response, "Response did not indicate client error");
    }

    #[actix_web::test]
    async fn creating_an_auth_key_requires_basic_authorization_header() {
        let app = test::init_service(build_app!()).await;

        // No Authorization header at all.
        let request = test::TestRequest::post().uri("/auth_keys").to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "POST /auth_keys without an Authorization header did not return 401."
        );
        assert_basic_challenge(&response, "A missing header got no challenge.");

        // An auth key is not a credential pair, so the wrong scheme is refused.
        let request = test::TestRequest::post()
            .uri("/auth_keys")
            .insert_header((AUTHORIZATION, "Bearer some-auth-key"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "POST /auth_keys with a Bearer header did not return 401."
        );
        assert_basic_challenge(&response, "A wrong scheme got no challenge.");

        // A token which is not base64.
        let request = test::TestRequest::post()
            .uri("/auth_keys")
            .insert_header((AUTHORIZATION, "Basic not-valid-base64!"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "POST /auth_keys with an undecodable token did not return 401."
        );
        assert_basic_challenge(&response, "An undecodable token got no challenge.");

        // Valid base64, but the decoded text has no colon to split on.
        let request = test::TestRequest::post()
            .uri("/auth_keys")
            .insert_header((
                AUTHORIZATION,
                format!("Basic {}", STANDARD.encode("no-colon")),
            ))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "POST /auth_keys with a token missing a colon did not return 401."
        );
        assert_basic_challenge(&response, "A token missing a colon got no challenge.");
    }

    #[actix_web::test]
    async fn refused_credentials_are_challenged_too() {
        let username = "refused-credentials-username";
        let password = "refused-credentials-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, username, password).await,
            "Unable to register account",
        );

        // The credentials were supplied and then refused, which RFC 9110 still
        // covers with a challenge.
        let wrong_password = create_auth_key(&app, username, "not-the-correct-password").await;
        assert_unauthorized(&wrong_password, "A wrong password did not return 401.");
        assert_basic_challenge(&wrong_password, "Refused credentials got no challenge.");

        // An unknown username is refused in exactly the same way. It is not a
        // 404: this endpoint and the protection space it guards both exist, so
        // the client is told to retry with credentials instead of being left
        // to guess whether it addressed the wrong URI.
        let unknown_username = create_auth_key(&app, "no-such-username", password).await;
        assert_unauthorized(
            &unknown_username,
            "Credentials for an unknown user did not return 401.",
        );
        assert_basic_challenge(&unknown_username, "An unknown username got no challenge.");
    }

    #[actix_web::test]
    async fn bearer_endpoints_challenge_a_missing_or_malformed_header() {
        let app = test::init_service(build_app!()).await;

        // No Authorization header at all: no auth key reached the server to be
        // judged, so the challenge names the request itself as what to fix.
        let request = test::TestRequest::get().uri("/whoami").to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "/whoami without an Authorization header did not return 401."
        );
        assert_only_challenge(
            &response,
            super::BEARER_INVALID_REQUEST_CHALLENGE,
            "A missing header got no challenge naming the bearer scheme.",
        );

        // A malformed header is rejected by the extractor, so it never reaches
        // a handler and is answered the same way as a missing one.
        for header_value in [
            "Basic some-auth-key",      // wrong scheme
            "some-auth-key",            // no scheme at all
            "Bearer",                   // no key
            "Bearer some-auth-key and", // trailing word
        ] {
            let request = test::TestRequest::get()
                .uri("/whoami")
                .insert_header((AUTHORIZATION, header_value))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(
                response.status().as_u16(),
                401,
                "/whoami with {header_value:?} did not return 401."
            );
            assert_only_challenge(
                &response,
                super::BEARER_INVALID_REQUEST_CHALLENGE,
                &format!("A malformed header {header_value:?} got no challenge."),
            );
        }
    }

    #[actix_web::test]
    async fn bearer_endpoints_challenge_a_refused_auth_key() {
        let app = test::init_service(build_app!()).await;

        // A well-formed header naming a key which is unknown or expired: the
        // key was presented and then refused, so the client needs a new one.
        let request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, "Bearer this-is-not-a-valid-auth-key"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "/whoami with an unknown auth key did not return 401."
        );
        assert_only_challenge(
            &response,
            super::BEARER_INVALID_TOKEN_CHALLENGE,
            "A refused auth key got no challenge telling the client to get new credentials.",
        );
    }

    #[actix_web::test]
    async fn bearer_endpoints_challenge_a_caller_without_access() {
        let owner_username = "challenged-owner-username";
        let stranger_username = "challenged-stranger-username";
        let password = "challenged-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "private task", None).await;

        let stranger_key = register_and_create_auth_key(&app, stranger_username, password).await;
        let request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {stranger_key}")))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            403,
            "Reading someone else's task did not return 403."
        );
        // The key was accepted, so the challenge names the reason the request
        // was refused rather than asking for a new key: another one would be
        // refused the same way.
        assert_insufficient_scope_challenge(
            &response,
            "A caller without access got no insufficient_scope challenge.",
        );
    }

    /// A password is separated from the username by the first colon and nothing
    /// else, so one may hold colons of its own. Only the endpoint which takes
    /// credentials ever sees one, and it still does at `POST /auth_keys`.
    #[actix_web::test]
    async fn login_password_may_contain_a_colon() {
        let username = "colon-password-username";
        let password = "colon:password:with:colons";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let whoami_response = assert_response_success(
            test::call_service(&app, whoami_request).await,
            "Whoami request failed.",
        );
        assert_eq!(extract_json_string(whoami_response, "username"), username);
    }

    /// An auth key is created at a URI of its own, so the creation has to say
    /// which one. The `Location` names it by the id of its row and the body
    /// carries the key, because those are two different things: the id
    /// identifies the key without being a credential, and the key authenticates
    /// without identifying anything.
    #[actix_web::test]
    async fn a_created_auth_key_is_at_the_uri_it_reports() {
        let username = "auth-key-location-username";
        let password = "auth-key-location-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, username, password).await,
            "Unable to register account",
        );

        let created = assert_response_success(
            create_auth_key(&app, username, password).await,
            "Could not create an auth key.",
        );
        assert_eq!(
            created.status().as_u16(),
            201,
            "Credentials which were accepted create an auth key."
        );

        let location = created
            .headers()
            .get(LOCATION)
            .expect("A 201 must name the auth key it created.")
            .to_str()
            .expect("A Location is ASCII.")
            .to_string();
        let auth_key = extract_json_string(created, "auth_key");
        let id = location
            .rsplit('/')
            .next()
            .expect("A Location ends in the id of the key it names.");
        assert_eq!(
            location,
            format!("/auth_keys/{id}"),
            "The Location of a created auth key is the URI of that key."
        );

        // The key is a bearer token, and RFC 6750 §5.3 keeps those out of URLs:
        // every proxy and log which records a request line would record the
        // credential. The id names the key; it is not the key.
        assert_ne!(
            id, auth_key,
            "The auth key itself is a credential and must not be in a URI."
        );

        // The key the client was handed is one the server accepts.
        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let whoami_response = assert_response_success(
            test::call_service(&app, whoami_request).await,
            "The created auth key does not work.",
        );
        assert_eq!(extract_json_string(whoami_response, "username"), username);
    }

    /// Revoking an auth key removes the one the caller presented, and the key
    /// stops working at once: a Bearer-protected endpoint answers 401 with the
    /// challenge which tells the client to get a new one.
    #[actix_web::test]
    async fn can_revoke_an_auth_key() {
        let username = "can-revoke-auth-key-username";
        let password = "can-revoke-auth-key-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let revoke_request = test::TestRequest::delete()
            .uri("/auth_keys")
            .insert_header((AUTHORIZATION, format!("bearer {auth_key}")))
            .to_request();
        let revoke_response = assert_response_success(
            test::call_service(&app, revoke_request).await,
            "Unable to revoke an auth key.",
        );
        assert_eq!(
            revoke_response.status().as_u16(),
            204,
            "A revoked auth key has nothing left to say about it."
        );

        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let whoami_response = test::call_service(&app, whoami_request).await;
        assert_unauthorized(
            &whoami_response,
            "A revoked auth key should not authenticate anything.",
        );
        assert_only_challenge(
            &whoami_response,
            super::BEARER_INVALID_TOKEN_CHALLENGE,
            "A revoked auth key got no challenge telling the client to get new credentials.",
        );
    }

    /// `DELETE` is idempotent, so revoking a key which is not there asks for the
    /// state the caller already has and is answered exactly as the removal
    /// which got it there was.
    #[actix_web::test]
    async fn revoking_an_auth_key_which_is_not_there_is_a_204() {
        let app = test::init_service(build_app!()).await;

        for _ in 0..2 {
            let revoke_request = test::TestRequest::delete()
                .uri("/auth_keys")
                .insert_header((AUTHORIZATION, "Bearer this-key-was-never-issued"))
                .to_request();
            let revoke_response = test::call_service(&app, revoke_request).await;
            assert_eq!(
                revoke_response.status().as_u16(),
                204,
                "Revoking an absent auth key leaves the state asked for."
            );
        }
    }

    /// An auth key is revoked by presenting it, never by naming it. There is
    /// nothing to gain from `DELETE /auth_keys/{key}`: the API hands out no key
    /// but the one the caller holds, so the URI could only ever name that same
    /// key, and a URI which said nothing the header does not is not one to
    /// create.
    #[actix_web::test]
    async fn an_auth_key_cannot_be_revoked_by_uri() {
        let username = "auth-key-not-by-uri-username";
        let password = "auth-key-not-by-uri-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let revoke_request = test::TestRequest::delete()
            .uri(&format!("/auth_keys/{auth_key}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let revoke_response = test::call_service(&app, revoke_request).await;
        assert_eq!(
            revoke_response.status().as_u16(),
            404,
            "There is no auth key item URI, so this names no resource."
        );

        // The key is untouched: a 404 removed nothing.
        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        assert_response_success(
            test::call_service(&app, whoami_request).await,
            "The refused URI revoked the auth key anyway.",
        );
    }

    #[actix_web::test]
    async fn can_add_task() {
        let username = String::from("can-add-task-username");
        let password = String::from("can-add-task-password");
        let app = test::init_service(build_app!()).await;

        let auth_key =
            register_and_create_auth_key(&app, username.as_str(), password.as_str()).await;

        assert_response_success(
            add_task(&app, &auth_key, "test title", Some("test description")).await,
            "Unable to add task.",
        );
    }

    #[actix_web::test]
    async fn added_task_is_at_the_uri_it_reports() {
        let username = String::from("added-task-location-username");
        let password = String::from("added-task-location-password");
        let app = test::init_service(build_app!()).await;

        let auth_key =
            register_and_create_auth_key(&app, username.as_str(), password.as_str()).await;

        let add_task_response =
            add_task(&app, &auth_key, "test title", Some("test description")).await;
        let created = assert_response_success(add_task_response, "Unable to add task.");

        // The creation names the URI of what it created, so the client does not
        // have to build one out of an id it found somewhere else.
        let location = created
            .headers()
            .get(LOCATION)
            .expect("A 201 must name the resource it created.")
            .to_str()
            .expect("A Location is ASCII.")
            .to_string();

        // The created representation and the URI it named agree on the id, and
        // that URI returns the task.
        let created_task = extract_json_from_constructor(created, "task", Task::from_json_object)
            .expect("The 201 must carry the created task.");
        assert_eq!(location, format!("/tasks/{}", created_task.id()));

        let get_task_request = test::TestRequest::get()
            .uri(&location)
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_task_response = assert_response_success(
            test::call_service(&app, get_task_request).await,
            "The Location of a created task should return it.",
        );
        let task = extract_json_from_constructor(get_task_response, "task", Task::from_json_object)
            .unwrap();
        assert_eq!(*task.id(), *created_task.id());
        assert_eq!(*task.title(), "test title");
    }

    #[actix_web::test]
    async fn cannot_add_task_without_valid_auth_key() {
        let app = test::init_service(build_app!()).await;
        let response = add_task(
            &app,
            "this-is-not-a-valid-auth-key",
            "test title",
            Some("test description"),
        )
        .await;
        // A key which is unknown or expired is an authentication failure, not
        // an authorization one, so this stays a 401 that asks for a new key.
        assert_unauthorized(
            &response,
            "Adding a task with an unknown auth key did not return 401.",
        );
        assert_only_challenge(
            &response,
            super::BEARER_INVALID_TOKEN_CHALLENGE,
            "An unknown auth key got no challenge telling the client to get new credentials.",
        );
    }

    #[actix_web::test]
    async fn can_get_task() {
        let username = String::from("can-get-task-username");
        let password = String::from("can-get-task-password");
        let app = test::init_service(build_app!()).await;

        let auth_key =
            register_and_create_auth_key(&app, username.as_str(), password.as_str()).await;

        let task_id = create_task(&app, &auth_key, "test title", Some("test description")).await;
        let get_task_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_task_response = assert_response_success(
            test::call_service(&app, get_task_request).await,
            "Unable to get task.",
        );
        let task = extract_json_from_constructor(get_task_response, "task", Task::from_json_object);
        assert_eq!(*task.unwrap().id(), task_id);
    }

    #[actix_web::test]
    async fn cannot_get_different_users_task() {
        let owner_username = "cannot-get-different-users-task-owner-username";
        let owner_password = "cannot-get-different-users-task-owner-password";
        let non_owner_username = "cannot-get-different-users-task-non-owner-username";
        let non_owner_password = "cannot-get-different-users-task-non-owner-password";
        let app = test::init_service(build_app!()).await;

        let owner_auth_key =
            register_and_create_auth_key(&app, owner_username, owner_password).await;

        let task_id = create_task(
            &app,
            &owner_auth_key,
            "test title",
            Some("test description"),
        )
        .await;

        let non_owner_auth_key =
            register_and_create_auth_key(&app, non_owner_username, non_owner_password).await;

        let get_task_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {non_owner_auth_key}")))
            .to_request();
        let get_task_response = test::call_service(&app, get_task_request).await;
        assert_forbidden(
            &get_task_response,
            "Reading a task you hold no share on should be a 403.",
        );
    }

    #[actix_web::test]
    async fn cannot_get_nonexistent_task() {
        let username = "cannot-get-nonexistent-task-username";
        let password = "cannot-get-nonexistent-task-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let get_task_request = test::TestRequest::get()
            .uri("/tasks/-1")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_task_response = test::call_service(&app, get_task_request).await;
        // A missing task must look the same as a task owned by someone else:
        // 403, not 404.
        assert_forbidden(
            &get_task_response,
            "Getting a nonexistent task should not reveal its nonexistence.",
        );
    }

    /// A task is named by its URI, so a path which cannot name one is a missing
    /// resource rather than a malformed request: the router answers 404, where
    /// the old query string asked a 400 of a request which named nothing.
    ///
    /// `GET /tasks` is not among them: it names the collection rather than one
    /// task, and its own filter is a client error rather than a missing path.
    #[actix_web::test]
    async fn a_path_which_cannot_name_a_task_is_not_found() {
        let username = "unusable-task-path-username";
        let password = "unusable-task-path-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        for (method, uri) in [
            (Method::GET, "/tasks/"),
            (Method::GET, "/tasks/not-a-number"),
            (Method::GET, "/tasks/1/"),
            (Method::PATCH, "/tasks"),
            (Method::PATCH, "/tasks/"),
            (Method::PATCH, "/tasks/not-a-number"),
        ] {
            let request = test::TestRequest::default()
                .method(method)
                .uri(uri)
                .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(
                response.status().as_u16(),
                404,
                "{uri} names no task, so it should be a 404."
            );
        }
    }

    #[actix_web::test]
    async fn cannot_update_nonexistent_task() {
        let username = "cannot-update-nonexistent-task-username";
        let password = "cannot-update-nonexistent-task-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let update_request = test::TestRequest::patch()
            .uri("/tasks/-1")
            .set_json(super::UpdateTaskPayload {
                completed: Some(true),
                title: None,
                description: None,
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_forbidden(
            &update_response,
            "Updating a nonexistent task should not reveal its nonexistence.",
        );
    }

    /// An empty body changes nothing, so it is a client error rather than a
    /// request which is quietly accepted and ignored.
    #[actix_web::test]
    async fn cannot_update_a_task_without_any_fields() {
        let username = "cannot-update-without-fields-username";
        let password = "cannot-update-without-fields-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let task_id = create_task(&app, &auth_key, "test title", None).await;

        let update_request = test::TestRequest::patch()
            .uri(&format!("/tasks/{task_id}"))
            .set_json(super::UpdateTaskPayload::default())
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(
            update_response.status().as_u16(),
            400,
            "A PATCH with no fields to change should be a 400."
        );
        assert_eq!(
            extract_json_string(update_response, "message"),
            "At least one field must be provided"
        );
    }

    // TODO: Return the updated task so it can be verified
    async fn update_task(
        username: &str,
        password: &str,
        completed: Option<bool>,
        title: Option<String>,
        description: Option<String>,
    ) -> Task {
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let task_id = create_task(&app, &auth_key, "test title", Some("test description")).await;

        let update_request = test::TestRequest::patch()
            .uri(&format!("/tasks/{task_id}"))
            .set_json(super::UpdateTaskPayload {
                completed,
                title,
                description,
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "Unable to update task.",
        );

        let get_task_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_task_response = assert_response_success(
            test::call_service(&app, get_task_request).await,
            "Unable to get task after creating.",
        );
        extract_json_from_constructor(get_task_response, "task", Task::from_json_object).unwrap()
    }

    #[actix_web::test]
    async fn title_is_updatable() {
        let updated_title = String::from("new title");
        let updated_task = update_task(
            "title-is-updateable-username",
            "title-is-updateable-password",
            None,
            Some(updated_title.clone()),
            None,
        )
        .await;
        assert_eq!(*updated_task.title(), updated_title)
    }

    #[actix_web::test]
    async fn description_is_updatable() {
        let updated_description = String::from("new description");
        let updated_task = update_task(
            "description-is-updateable-username",
            "description-is-updateable-password",
            None,
            None,
            Some(updated_description.clone()),
        )
        .await;
        assert_eq!(
            *updated_task.description().as_ref().unwrap(),
            updated_description
        )
    }

    #[actix_web::test]
    async fn completed_is_updatable() {
        let updated_completed = true;
        let updated_task = update_task(
            "completed-is-updateable-username",
            "completed-is-updateable-password",
            Some(updated_completed),
            None,
            None,
        )
        .await;
        assert!(updated_task.completed())
    }

    #[actix_web::test]
    async fn update_modifies_only_the_target_task() {
        let username = "update-only-target-username";
        let password = "update-only-target-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let first_task_id = create_task(&app, &auth_key, "first task", None).await;
        let second_task_id = create_task(&app, &auth_key, "second task", None).await;

        let update_request = test::TestRequest::patch()
            .uri(&format!("/tasks/{first_task_id}"))
            .set_json(super::UpdateTaskPayload {
                completed: Some(true),
                title: None,
                description: None,
            })
            .insert_header((AUTHORIZATION, format!("Bearer {}", auth_key.clone())))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "Unable to update first task.",
        );

        let get_second = test::TestRequest::get()
            .uri(&format!("/tasks/{second_task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_second_response = assert_response_success(
            test::call_service(&app, get_second).await,
            "Unable to get second task.",
        );
        let second_task =
            extract_json_from_constructor(get_second_response, "task", Task::from_json_object)
                .unwrap();
        assert!(
            !second_task.completed(),
            "Updating one task changed another task's completion state"
        );
        assert_eq!(
            *second_task.title(),
            "second task",
            "Updating one task changed another task's title"
        );
    }

    /// Without a `completed` filter, the collection is everything the caller can
    /// see: the tasks they own plus the tasks shared with them.
    #[actix_web::test]
    async fn can_get_tasks() {
        let username = "can-get-tasks-username";
        let password = "can-get-tasks-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        assert_response_success(
            add_task(&app, &auth_key, "task 1", None).await,
            "Unable to add first task.",
        );

        assert_response_success(
            add_task(&app, &auth_key, "task 2", None).await,
            "Unable to add second task.",
        );

        let get_tasks_response =
            assert_response_success(get_tasks(&app, &auth_key, "").await, "Could not get tasks!");
        let tasks: Vec<Task> = extract_json_tasks(get_tasks_response);
        assert_eq!(tasks.len(), 2, "Expected 2 tasks, found {}", tasks.len());
    }

    /// The completed filter narrows the collection rather than replacing it:
    /// both values of it answer a view of the same URI.
    #[actix_web::test]
    async fn the_completed_filter_selects_incomplete_tasks() {
        let username = "completed-filter-incomplete-username";
        let password = "completed-filter-incomplete-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        assert_response_success(
            add_task(&app, &auth_key, "task 1", None).await,
            "Unable to add first task.",
        );

        let second_task_id = create_task(&app, &auth_key, "task 2", None).await;

        assert_response_success(
            set_task_completed(&app, &auth_key, second_task_id, true).await,
            "Unable to mark second task completed.",
        );

        let get_tasks_response = assert_response_success(
            get_tasks(&app, &auth_key, "?completed=false").await,
            "Could not get incomplete tasks!",
        );
        let tasks: Vec<Task> = extract_json_tasks(get_tasks_response);
        assert_eq!(
            tasks.len(),
            1,
            "Expected 1 incomplete task, found {}",
            tasks.len()
        );
        assert_eq!(*tasks[0].title(), "task 1");
    }

    /// The other value of the same filter answers the other view, so a client
    /// needs no second URI to ask for the completed tasks.
    #[actix_web::test]
    async fn the_completed_filter_selects_completed_tasks() {
        let username = "completed-filter-completed-username";
        let password = "completed-filter-completed-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        assert_response_success(
            add_task(&app, &auth_key, "task 1", None).await,
            "Unable to add first task.",
        );

        let second_task_id = create_task(&app, &auth_key, "task 2", None).await;

        assert_response_success(
            set_task_completed(&app, &auth_key, second_task_id, true).await,
            "Unable to mark second task completed.",
        );

        let get_tasks_response = assert_response_success(
            get_tasks(&app, &auth_key, "?completed=true").await,
            "Could not get completed tasks!",
        );
        let tasks: Vec<Task> = extract_json_tasks(get_tasks_response);
        assert_eq!(
            tasks.len(),
            1,
            "Expected 1 completed task, found {}",
            tasks.len()
        );
        assert_eq!(*tasks[0].title(), "task 2");
    }

    /// A `completed` value which is not a boolean names no view of the
    /// collection, so it is answered as a client error carrying the standard
    /// JSON error body rather than silently matching nothing.
    #[actix_web::test]
    async fn a_non_boolean_completed_filter_is_a_bad_request() {
        let username = "bad-completed-filter-username";
        let password = "bad-completed-filter-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        for uri in ["/tasks?completed=maybe", "/tasks?completed="] {
            let request = test::TestRequest::get()
                .uri(uri)
                .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status().as_u16(), 400, "{uri} should be a 400.");
            assert_json_error_with_request_id(response);
        }

        // The credentials are settled before the filter is read, so a request
        // carrying neither is still refused as unauthenticated rather than
        // told what was wrong with its query string.
        let request = test::TestRequest::get()
            .uri("/tasks?completed=maybe")
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_unauthorized(
            &response,
            "An unreadable filter must not answer a request which holds no credentials",
        );
    }

    /// The filter is answered from the database, so a task completed by someone
    /// else leaves the caller's filtered view even though the caller still
    /// holds a share on it and still sees it unfiltered.
    #[actix_web::test]
    async fn completing_a_shared_task_removes_it_from_the_sharees_incomplete_view() {
        let owner_username = "shared-completed-owner-username";
        let grantee_username = "shared-completed-grantee-username";
        let password = "shared-completed-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let grantee_key = register_and_create_auth_key(&app, grantee_username, password).await;

        let task_id = create_task(&app, &owner_key, "shared task", None).await;
        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        // The sharee sees the task while it is incomplete...
        let response = assert_response_success(
            get_tasks(&app, &grantee_key, "?completed=false").await,
            "list sharee incomplete tasks",
        );
        let tasks: Vec<Task> = extract_json_tasks(response);
        assert_eq!(tasks.len(), 1);

        // ...and stops seeing it once the owner completes it, without losing
        // it from the collection as a whole.
        assert_response_success(
            set_task_completed(&app, &owner_key, task_id, true).await,
            "owner should be able to complete the shared task",
        );

        let filtered = assert_response_success(
            get_tasks(&app, &grantee_key, "?completed=false").await,
            "list sharee incomplete tasks",
        );
        assert_eq!(
            extract_json_tasks(filtered).len(),
            0,
            "A completed task should leave the sharee's incomplete view"
        );

        let unfiltered =
            assert_response_success(get_tasks(&app, &grantee_key, "").await, "list sharee tasks");
        let tasks: Vec<Task> = extract_json_tasks(unfiltered);
        assert_eq!(
            tasks.len(),
            1,
            "The sharee should still see the completed task unfiltered"
        );
        assert_eq!(*tasks[0].id(), task_id);
    }

    /// Assigns a permission to a share at the share's own URI, the way a
    /// client which knows the task id and the username does.
    async fn share_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
        username: &str,
        permission: &str,
    ) -> ServiceResponse {
        let request = test::TestRequest::put()
            .uri(&format!("/tasks/{id}/shares/{username}"))
            .set_json(super::SharePayload {
                permission: String::from(permission),
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    /// Removes a share at the share's own URI. There is no body to send: the
    /// URI names everything the request acts on.
    async fn unshare_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
        username: &str,
    ) -> ServiceResponse {
        let request = test::TestRequest::delete()
            .uri(&format!("/tasks/{id}/shares/{username}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    async fn get_task_shares(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
    ) -> ServiceResponse {
        let request = test::TestRequest::get()
            .uri(&format!("/tasks/{id}/shares"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    async fn get_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
    ) -> ServiceResponse {
        let request = test::TestRequest::get()
            .uri(&format!("/tasks/{id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    async fn set_task_completed(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
        completed: bool,
    ) -> ServiceResponse {
        let request = test::TestRequest::patch()
            .uri(&format!("/tasks/{id}"))
            .set_json(super::UpdateTaskPayload {
                completed: Some(completed),
                title: None,
                description: None,
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    #[actix_web::test]
    async fn owner_can_share_and_see_share_listed() {
        let owner_username = "share-list-owner-username";
        let grantee_username = "share-list-grantee-username";
        let password = "share-list-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );

        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0]["username"].as_str().unwrap(), grantee_username);
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read");
    }

    /// A share is created at a URI of its own, so the creation has to say which
    /// one: the `Location` names the share the client just made, and the
    /// representation in the body is the share itself.
    #[actix_web::test]
    async fn a_created_share_is_at_the_uri_it_reports() {
        let owner_username = "share-location-owner-username";
        let grantee_username = "share-location-grantee-username";
        let password = "share-location-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        let created = assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );
        assert_eq!(
            created.status().as_u16(),
            201,
            "A share which did not exist is created."
        );

        let location = created
            .headers()
            .get(LOCATION)
            .expect("A 201 must name the share it created.")
            .to_str()
            .expect("A Location is ASCII.")
            .to_string();
        assert_eq!(
            location,
            format!("/tasks/{task_id}/shares/{grantee_username}"),
            "The Location of a created share is the URI of that share."
        );

        let share = extract_json_from_constructor(created, "share", |v| v.clone());
        assert_eq!(share["username"].as_str().unwrap(), grantee_username);
        assert_eq!(share["permission"].as_str().unwrap(), "read");
        assert!(
            share["id"].is_i64(),
            "A share carries its own id: {share:?}"
        );
    }

    /// The status is how a client tells a new share from a changed one, so the
    /// same request has to answer differently depending on which it was.
    #[actix_web::test]
    async fn resharing_changes_the_permission() {
        let owner_username = "reshare-owner-username";
        let grantee_username = "reshare-grantee-username";
        let password = "reshare-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        let created = share_task(&app, &owner_key, task_id, grantee_username, "read").await;
        assert_eq!(
            created.status().as_u16(),
            201,
            "The first share created a share."
        );

        let replaced = share_task(&app, &owner_key, task_id, grantee_username, "read_write").await;
        assert_eq!(
            replaced.status().as_u16(),
            200,
            "The second share replaced the permission on the share already there."
        );

        // Nothing was created the second time, so there is nothing to point at.
        assert!(
            replaced.headers().get(LOCATION).is_none(),
            "A replaced share is not a new resource, so it names no Location."
        );

        let replaced_share = extract_json_from_constructor(replaced, "share", |v| v.clone());
        assert_eq!(replaced_share["permission"].as_str().unwrap(), "read_write");

        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(shares.len(), 1, "Re-sharing should replace, not duplicate");
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read_write");
    }

    /// The point of `PUT` is that repeating it is safe: the second request
    /// finds the share the first one made, says it replaced what was there,
    /// and leaves one share rather than two.
    #[actix_web::test]
    async fn repeating_a_share_with_the_same_permission_is_safe() {
        let owner_username = "repeat-share-owner-username";
        let grantee_username = "repeat-share-grantee-username";
        let password = "repeat-share-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        for _ in 0..2 {
            assert_response_success(
                share_task(&app, &owner_key, task_id, grantee_username, "read").await,
                "Share request failed",
            );
        }

        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(
            shares.len(),
            1,
            "The same permission assigned twice is still one share."
        );
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read");
    }

    /// A permission which is neither of the two the API knows names no state a
    /// share could be in, so it is a client error rather than a share quietly
    /// given a permission nobody can use.
    #[actix_web::test]
    async fn an_unusable_permission_is_a_bad_request() {
        let owner_username = "bad-permission-owner-username";
        let grantee_username = "bad-permission-grantee-username";
        let password = "bad-permission-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        for permission in ["", "owner", "read-write", "READ"] {
            let response =
                share_task(&app, &owner_key, task_id, grantee_username, permission).await;
            assert_eq!(
                response.status().as_u16(),
                400,
                "{permission:?} is not a permission this API knows."
            );
        }

        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        assert_eq!(
            extract_shares(list_response).len(),
            0,
            "A refused permission must not have created a share."
        );
    }

    #[actix_web::test]
    async fn non_owner_cannot_share_unshare_or_list() {
        let owner_username = "nonowner-share-owner-username";
        let non_owner_username = "nonowner-share-nonowner-username";
        let password = "nonowner-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let non_owner_key = register_and_create_auth_key(&app, non_owner_username, password).await;

        let task_id = create_task(&app, &owner_key, "task", None).await;

        assert_forbidden(
            &share_task(&app, &non_owner_key, task_id, non_owner_username, "read").await,
            "Sharing someone else's task should be a 403.",
        );
        assert_forbidden(
            &unshare_task(&app, &non_owner_key, task_id, non_owner_username).await,
            "Unsharing someone else's task should be a 403.",
        );
        assert_forbidden(
            &get_task_shares(&app, &non_owner_key, task_id).await,
            "Listing the shares on someone else's task should be a 403.",
        );
    }

    #[actix_web::test]
    async fn unsharing_removes_the_row() {
        let owner_username = "unshare-owner-username";
        let grantee_username = "unshare-grantee-username";
        let password = "unshare-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "task", None).await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );
        assert_response_success(
            unshare_task(&app, &owner_key, task_id, grantee_username).await,
            "Unshare request failed",
        );

        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(shares.len(), 0, "Share row should have been removed");
    }

    /// `DELETE` is idempotent, so removing a share which is already gone asks
    /// for the state the caller has, and is answered the same way as the
    /// removal which got it there. A client which repeats the request — after
    /// a timeout, say — learns it worked either time.
    #[actix_web::test]
    async fn unsharing_a_share_which_is_not_there_is_a_204() {
        let owner_username = "unshare-absent-owner-username";
        let grantee_username = "unshare-absent-grantee-username";
        let password = "unshare-absent-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(
            register(&app, grantee_username, password).await,
            "Unable to register grantee",
        );

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "task", None).await;

        // Never shared at all.
        assert_response_success(
            unshare_task(&app, &owner_key, task_id, grantee_username).await,
            "Removing a share which was never there leaves the state asked for.",
        );

        // Shared, then removed twice.
        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );
        let first = unshare_task(&app, &owner_key, task_id, grantee_username).await;
        assert_eq!(
            first.status().as_u16(),
            204,
            "The share was there to remove."
        );
        let repeated = unshare_task(&app, &owner_key, task_id, grantee_username).await;
        assert_eq!(
            repeated.status().as_u16(),
            first.status().as_u16(),
            "Repeating the removal must answer exactly as the first one did."
        );
        assert!(
            repeated
                .headers()
                .get(actix_web::http::header::CONTENT_TYPE)
                .is_none(),
            "204 carries no representation, so it names no media type."
        );
    }

    #[actix_web::test]
    async fn unknown_target_username_gives_404() {
        let owner_username = "unknown-target-owner-username";
        let password = "unknown-target-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let task_id = create_task(&app, &owner_key, "task", None).await;

        assert_eq!(
            share_task(&app, &owner_key, task_id, "no-such-user", "read")
                .await
                .status()
                .as_u16(),
            404
        );
        assert_eq!(
            unshare_task(&app, &owner_key, task_id, "no-such-user")
                .await
                .status()
                .as_u16(),
            404
        );
    }

    #[actix_web::test]
    async fn missing_task_returns_same_403_as_not_owned() {
        let owner_username = "oracle-owner-username";
        let other_username = "oracle-other-username";
        let password = "oracle-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let other_key = register_and_create_auth_key(&app, other_username, password).await;

        let task_id = create_task(&app, &owner_key, "task", None).await;

        // Non-owner trying to manage an existing task
        let not_owned = share_task(&app, &other_key, task_id, other_username, "read").await;
        // Anyone trying to manage a missing task
        let missing = share_task(&app, &owner_key, -1, other_username, "read").await;
        assert_forbidden(&not_owned, "Sharing someone else's task should be a 403.");
        assert_forbidden(&missing, "Sharing a missing task should be a 403.");
        assert_eq!(
            json_body_keys(missing),
            json_body_keys(not_owned),
            "A missing task must be reported with the same body shape as one that is not yours."
        );

        let not_owned_list = get_task_shares(&app, &other_key, task_id).await;
        let missing_list = get_task_shares(&app, &owner_key, -1).await;
        assert_forbidden(
            &not_owned_list,
            "Listing shares on someone else's task should be a 403.",
        );
        assert_forbidden(
            &missing_list,
            "Listing shares on a missing task should be a 403.",
        );
        assert_eq!(
            json_body_keys(missing_list),
            json_body_keys(not_owned_list),
            "A missing task must be reported with the same body shape as one that is not yours."
        );

        let not_owned_unshare = unshare_task(&app, &other_key, task_id, other_username).await;
        let missing_unshare = unshare_task(&app, &owner_key, -1, other_username).await;
        assert_forbidden(
            &not_owned_unshare,
            "Unsharing someone else's task should be a 403.",
        );
        assert_forbidden(
            &missing_unshare,
            "Unsharing a missing task should be a 403.",
        );
        assert_eq!(
            json_body_keys(missing_unshare),
            json_body_keys(not_owned_unshare),
            "A missing task must be reported with the same body shape as one that is not yours."
        );

        let not_owned_get = get_task(&app, &other_key, task_id).await;
        let missing_get = get_task(&app, &owner_key, -1).await;
        assert_forbidden(
            &not_owned_get,
            "Reading someone else's task should be a 403.",
        );
        assert_forbidden(&missing_get, "Reading a missing task should be a 403.");
        assert_eq!(
            json_body_keys(missing_get),
            json_body_keys(not_owned_get),
            "A missing task must be reported with the same body shape as one that is not yours."
        );

        let not_owned_patch = set_task_completed(&app, &other_key, task_id, true).await;
        let missing_patch = set_task_completed(&app, &owner_key, -1, true).await;
        assert_forbidden(
            &not_owned_patch,
            "Updating someone else's task should be a 403.",
        );
        assert_forbidden(&missing_patch, "Updating a missing task should be a 403.");
        assert_eq!(
            json_body_keys(missing_patch),
            json_body_keys(not_owned_patch),
            "A missing task must be reported with the same body shape as one that is not yours."
        );
    }

    #[actix_web::test]
    async fn read_write_sharee_can_update_task_fields() {
        let owner_username = "rw-update-owner-username";
        let grantee_username = "rw-update-grantee-username";
        let password = "rw-update-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let grantee_key = register_and_create_auth_key(&app, grantee_username, password).await;

        let task_id = create_task(
            &app,
            &owner_key,
            "shared task",
            Some("original description"),
        )
        .await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        let update_request = test::TestRequest::patch()
            .uri(&format!("/tasks/{task_id}"))
            .set_json(super::UpdateTaskPayload {
                completed: Some(true),
                title: Some(String::from("updated by grantee")),
                description: Some(String::from("updated description")),
            })
            .insert_header((AUTHORIZATION, format!("Bearer {}", grantee_key.clone())))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "read_write grantee should be able to update the task",
        );

        let get_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {owner_key}")))
            .to_request();
        let get_response =
            assert_response_success(test::call_service(&app, get_request).await, "get task");
        let task =
            extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert!(*task.completed());
        assert_eq!(*task.title(), "updated by grantee");
        assert_eq!(task.description().as_ref().unwrap(), "updated description");
    }

    #[actix_web::test]
    async fn read_only_sharee_cannot_update_task() {
        let owner_username = "ro-update-owner-username";
        let grantee_username = "ro-update-grantee-username";
        let password = "ro-update-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let grantee_key = register_and_create_auth_key(&app, grantee_username, password).await;

        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );

        let update_request = test::TestRequest::patch()
            .uri(&format!("/tasks/{task_id}"))
            .set_json(super::UpdateTaskPayload {
                completed: Some(true),
                title: None,
                description: None,
            })
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_forbidden(
            &update_response,
            "A read-only sharee must get a 403 when updating the task.",
        );

        // The task itself must be unchanged.
        let get_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {owner_key}")))
            .to_request();
        let get_response =
            assert_response_success(test::call_service(&app, get_request).await, "get task");
        let task =
            extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert!(
            !task.completed(),
            "read-only sharee's update must not have been applied"
        );
        assert_eq!(*task.title(), "shared task");
    }

    #[actix_web::test]
    async fn unrelated_user_cannot_update_task() {
        let owner_username = "unrelated-update-owner-username";
        let stranger_username = "unrelated-update-stranger-username";
        let password = "unrelated-update-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let stranger_key = register_and_create_auth_key(&app, stranger_username, password).await;

        let task_id = create_task(&app, &owner_key, "private task", None).await;

        let update_request = test::TestRequest::patch()
            .uri(&format!("/tasks/{task_id}"))
            .set_json(super::UpdateTaskPayload {
                completed: Some(true),
                title: None,
                description: None,
            })
            .insert_header((AUTHORIZATION, format!("Bearer {stranger_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_forbidden(
            &update_response,
            "An unrelated user must get a 403 when updating the task.",
        );

        // The task itself must be unchanged.
        let get_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {owner_key}")))
            .to_request();
        let get_response =
            assert_response_success(test::call_service(&app, get_request).await, "get task");
        let task =
            extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert!(
            !task.completed(),
            "Unrelated user's update must not have been applied"
        );
        assert_eq!(*task.title(), "private task");
    }

    #[actix_web::test]
    async fn read_write_sharee_cannot_manage_shares() {
        let owner_username = "rw-manage-owner-username";
        let grantee_username = "rw-manage-grantee-username";
        let password = "rw-manage-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let grantee_key = register_and_create_auth_key(&app, grantee_username, password).await;

        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        let failed_share =
            share_task(&app, &grantee_key, task_id, owner_username, "read_write").await;
        assert_forbidden(
            &failed_share,
            "A read_write sharee must get a 403 when re-sharing the task.",
        );

        // The failed share call must not have created any share row.
        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(
            shares.len(),
            1,
            "Failed share call must not create a new share"
        );
        assert_eq!(shares[0]["username"].as_str().unwrap(), grantee_username);

        let failed_unshare = unshare_task(&app, &grantee_key, task_id, grantee_username).await;
        assert_forbidden(
            &failed_unshare,
            "A read_write sharee must get a 403 when unsharing the task.",
        );

        // The failed unshare call must not have removed the existing share.
        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(
            shares.len(),
            1,
            "Failed unshare call must not remove the existing share"
        );
        assert_eq!(shares[0]["username"].as_str().unwrap(), grantee_username);
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read_write");

        assert_forbidden(
            &get_task_shares(&app, &grantee_key, task_id).await,
            "A read_write sharee must get a 403 when listing the shares.",
        );
    }

    /// A share is named entirely by its path, so a path which cannot name one is
    /// a missing resource rather than a malformed request: the router answers
    /// 404, where the query string this endpoint used to take asked a 400 of a
    /// request which named nothing. There is no query string on these URIs to
    /// get wrong any more.
    #[actix_web::test]
    async fn a_path_which_cannot_name_a_share_is_not_found() {
        let username = "unusable-share-path-username";
        let password = "unusable-share-path-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;
        let task_id = create_task(&app, &auth_key, "task", None).await;

        for (method, uri) in [
            (Method::GET, format!("/tasks/{task_id}/shares/")),
            (Method::GET, "/tasks/not-a-number/shares".to_string()),
            (Method::PUT, format!("/tasks/{task_id}/shares/")),
            (
                Method::PUT,
                format!("/tasks/{task_id}/shares/{username}/extra"),
            ),
            (Method::DELETE, format!("/tasks/{task_id}/shares/")),
            (
                Method::DELETE,
                format!("/tasks/not-a-number/shares/{username}"),
            ),
        ] {
            let request = test::TestRequest::default()
                .method(method.clone())
                .uri(&uri)
                .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(
                response.status().as_u16(),
                404,
                "{method} {uri} names no share, so it should be a 404."
            );
        }
    }

    #[actix_web::test]
    async fn read_share_grants_get_and_listing_access() {
        let owner_username = "read-share-owner-username";
        let grantee_username = "read-share-grantee-username";
        let password = "read-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let grantee_key = register_and_create_auth_key(&app, grantee_username, password).await;

        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );

        // The grantee can now fetch the task directly...
        let get_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        let get_response = assert_response_success(
            test::call_service(&app, get_request).await,
            "grantee should be able to get the shared task",
        );
        let task =
            extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert_eq!(*task.id(), task_id);

        // ...and it shows up in their listings.
        let list_response = assert_response_success(
            get_tasks(&app, &grantee_key, "").await,
            "list grantee tasks",
        );
        let tasks: Vec<Task> = extract_json_tasks(list_response);
        assert_eq!(tasks.len(), 1);
        assert_eq!(*tasks[0].id(), task_id);

        let incomplete_response = assert_response_success(
            get_tasks(&app, &grantee_key, "?completed=false").await,
            "list grantee incomplete tasks",
        );
        let incomplete: Vec<Task> = extract_json_tasks(incomplete_response);
        assert_eq!(incomplete.len(), 1);
        assert_eq!(*incomplete[0].id(), task_id);
    }

    #[actix_web::test]
    async fn read_write_share_grants_get_and_listing_access() {
        let owner_username = "rw-share-owner-username";
        let grantee_username = "rw-share-grantee-username";
        let password = "rw-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let grantee_key = register_and_create_auth_key(&app, grantee_username, password).await;

        let task_id = create_task(&app, &owner_key, "shared task", None).await;

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        let get_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        assert_response_success(
            test::call_service(&app, get_request).await,
            "read_write grantee should be able to get the task",
        );

        let list_response = assert_response_success(
            get_tasks(&app, &grantee_key, "").await,
            "list grantee tasks",
        );
        let tasks: Vec<Task> = extract_json_tasks(list_response);
        assert_eq!(tasks.len(), 1);
        assert_eq!(*tasks[0].id(), task_id);
    }

    #[actix_web::test]
    async fn no_share_means_403_and_no_listing_entry() {
        let owner_username = "no-share-owner-username";
        let stranger_username = "no-share-stranger-username";
        let password = "no-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;
        let stranger_key = register_and_create_auth_key(&app, stranger_username, password).await;

        let task_id = create_task(&app, &owner_key, "private task", None).await;

        let get_request = test::TestRequest::get()
            .uri(&format!("/tasks/{task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {stranger_key}")))
            .to_request();
        let get_response = test::call_service(&app, get_request).await;
        assert_forbidden(
            &get_response,
            "A user with no share on a task must get a 403 when fetching it.",
        );

        let list_response = assert_response_success(
            get_tasks(&app, &stranger_key, "").await,
            "list stranger tasks",
        );
        let body: serde_json::Value =
            serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let tasks = body["tasks"].as_array().unwrap();
        assert_eq!(
            tasks.len(),
            0,
            "Stranger should not see the task in listings"
        );
    }

    #[actix_web::test]
    async fn owner_listings_never_duplicate_with_stale_share_row() {
        let owner_username = "stale-share-owner-username";
        let password = "stale-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_create_auth_key(&app, owner_username, password).await;

        let owned_task_id = create_task(&app, &owner_key, "owned task", None).await;

        // Simulate a stale share row granting the owner access to their own
        // task by inserting it directly; the owner's listing must not
        // duplicate the task.
        let mut connection = super::establish_connection().expect("db connection");
        let stale_owner = super::auth_key_to_user(&owner_key, &mut connection).unwrap();
        use crate::schema::task_shares::dsl::*;
        use diesel::prelude::*;
        diesel::insert_into(task_shares)
            .values((
                task_id.eq(&owned_task_id),
                user_id.eq(stale_owner.ref_id()),
                permission.eq(crate::core::SharePermission::Read),
            ))
            .execute(&mut connection)
            .expect("insert stale share row");
        drop(connection);

        let list_response =
            assert_response_success(get_tasks(&app, &owner_key, "").await, "list owner tasks");
        let tasks: Vec<Task> = extract_json_tasks(list_response);
        assert_eq!(
            tasks.len(),
            1,
            "Owner's task should appear exactly once even with a stale share row"
        );
    }

    #[actix_web::test]
    async fn user_by_id_returns_username_for_existing_user() {
        let username = "user-by-id-existing-username";
        let password = "user-by-id-existing-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        // Look up the user's id directly (usernames are the public handle;
        // task/share payloads carry the id).
        let user_id = {
            let mut connection = super::establish_connection().expect("db connection");
            let un = crate::domain_types::Username::new(String::from(username)).unwrap();
            let user = crate::core::get_user_by_name(&un, &mut connection).unwrap();
            *user.ref_id()
        };

        let request = test::TestRequest::get()
            .uri(&format!("/user_by_id?id={user_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let response = assert_response_success(
            test::call_service(&app, request).await,
            "User by id lookup failed.",
        );
        let response_name = extract_json_string(response, "username");
        assert_eq!(username, response_name);
    }

    #[actix_web::test]
    async fn user_by_id_returns_404_for_unknown_id() {
        let username = "user-by-id-unknown-username";
        let password = "user-by-id-unknown-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        let request = test::TestRequest::get()
            .uri("/user_by_id?id=-1")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            404,
            "Unknown user id should return 404."
        );
    }

    #[actix_web::test]
    async fn cannot_get_user_without_a_valid_id_query_parameter() {
        let username = "cannot-get-user-without-id-username";
        let password = "cannot-get-user-without-id-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        for uri in [
            "/user_by_id",
            "/user_by_id?id=",
            "/user_by_id?id=not-a-number",
        ] {
            let request = test::TestRequest::get()
                .uri(uri)
                .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status().as_u16(), 400, "{uri} should be a 400.");
            assert_json_error_with_request_id(response);
        }
    }

    #[actix_web::test]
    async fn json_bodies_are_declared_as_json() {
        let username = "json-media-type-username";
        let password = "json-media-type-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_create_auth_key(&app, username, password).await;

        // A success with a body.
        let add_task_response = assert_response_success(
            add_task(&app, &auth_key, "test title", Some("test description")).await,
            "Unable to add task.",
        );
        assert_json_content_type(&add_task_response, "/tasks said nothing about its body.");

        // A 400 from the query string extractor.
        let request = test::TestRequest::get()
            .uri("/user_by_id?id=not-a-number")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            400,
            "/user_by_id with an unreadable query string should be a 400."
        );
        assert_json_content_type(&response, "The 400 said nothing about its body.");

        // A 400 from the JSON body extractor.
        let request = test::TestRequest::post()
            .uri("/tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .insert_header((CONTENT_TYPE, "application/json"))
            .set_payload("this is not JSON")
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            400,
            "A malformed request body should be a 400."
        );
        assert_json_content_type(
            &response,
            "The malformed body 400 said nothing about its body.",
        );

        // A 401.
        let request = test::TestRequest::get().uri("/whoami").to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "/whoami without a key is a 401."
        );
        assert_json_content_type(&response, "The 401 said nothing about its body.");
    }

    #[actix_web::test]
    async fn responses_without_a_body_declare_no_media_type() {
        let username = "no-body-media-type-username";
        let password = "no-body-media-type-password";
        let app = test::init_service(build_app!()).await;

        let register_response = assert_response_success(
            register(&app, username, password).await,
            "Unable to register account",
        );
        assert!(
            register_response.headers().get(CONTENT_TYPE).is_none(),
            "An empty body has no representation to describe."
        );

        let created = assert_response_success(
            create_auth_key(&app, username, password).await,
            "Could not create an auth key.",
        );
        let auth_key = extract_json_string(created, "auth_key");

        let revoke_request = test::TestRequest::delete()
            .uri("/auth_keys")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let revoke_response = assert_response_success(
            test::call_service(&app, revoke_request).await,
            "Unable to revoke an auth key.",
        );
        assert!(
            revoke_response.headers().get(CONTENT_TYPE).is_none(),
            "An empty body has no representation to describe."
        );
    }
}
