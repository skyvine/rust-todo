use crate::core::{
    ApplicationError, Permission, SharePermission, add_share_for_task, add_user, auth_key_to_user,
    establish_connection, get_incomplete_tasks_for_user, get_new_auth_key, get_task_permission,
    get_user_by_name, list_shares_for_task, remove_share,
};
use crate::domain_types::{AuthKey, CleartextPassword, Credentials, TaskDescription, TaskTitle, Username};

use actix_web::{
    Error, FromRequest, HttpRequest, dev::Payload, error::InternalError, error::QueryPayloadError,
};
use actix_web::{HttpResponse, Responder, delete, get, patch, post};
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
fn error_body(request_id: impl std::fmt::Display) -> String {
    format!("{}", json!({"request_id": request_id.to_string()}))
}

fn error_body_with_message(request_id: impl std::fmt::Display, message: impl Serialize) -> String {
    format!(
        "{}",
        json!({"request_id": request_id.to_string(), "message": message})
    )
}

impl ApplicationError {
    fn into_http_response(self, request_id: &String) -> HttpResponse {
        match self {
            ApplicationError::DieselError(e) => {
                event!(Level::ERROR, "Diesel error: {e}");
                match e {
                    diesel::result::Error::NotFound => {
                        HttpResponse::NotFound().body(error_body(request_id))
                    }
                    _ => HttpResponse::InternalServerError().body(error_body(request_id)),
                }
            }
            ApplicationError::InvalidAuthKey => {
                event!(Level::ERROR, "Invalid auth key");
                HttpResponse::Unauthorized().body(error_body(request_id))
            }
            ApplicationError::InvalidData(message) => {
                event!(Level::ERROR, "Invalid data: {message}");
                HttpResponse::BadRequest().body(error_body_with_message(request_id, message))
            }
            ApplicationError::InvalidPassword => {
                event!(Level::ERROR, "Invalid password");
                HttpResponse::Unauthorized().body(error_body(request_id))
            }
            ApplicationError::QueryFailed(e) => {
                event!(Level::ERROR, "Query failed: {e}");
                HttpResponse::InternalServerError().body(error_body(request_id))
            }
            ApplicationError::Unauthorized => {
                event!(Level::ERROR, "Unauthorized");
                HttpResponse::Unauthorized().body(error_body(request_id))
            }
            ApplicationError::UserExists => HttpResponse::Conflict().body(error_body(request_id)),
        }
    }
}

// Shared request extractors

/// The scheme an `Authorization` header must use to carry an auth key.
const BEARER_SCHEME: &str = "Bearer";

/// The scheme an `Authorization` header must use to carry a username and a
/// password.
const BASIC_SCHEME: &str = "Basic";

/// The challenge which every 401 from `/login` carries, naming the protection
/// space this server guards.
///
/// RFC 7617 requires the realm and permits the optional charset, whose only
/// allowed value is UTF-8: that is the encoding this server reads credentials
/// as.
const LOGIN_BASIC_CHALLENGE: &str = "Basic realm=\"todo\", charset=\"UTF-8\"";

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
            event!(Level::ERROR, "Incorrect scheme \"{:?}\"", scheme);
        }

        if end.is_some() {
            event!(Level::ERROR, "Extra token after key: {:?}", end);
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

/// Turns a missing or malformed `Authorization` header into the same 401 an
/// invalid key gets, so a client cannot tell the two apart.
fn unauthorized_error(request_id: Uuid) -> Error {
    // The header itself is deliberately not logged: it may hold a valid key.
    event!(Level::ERROR, "Missing or malformed Authorization header");
    InternalError::from_response(
        "Missing or malformed Authorization header",
        HttpResponse::Unauthorized().body(error_body(request_id)),
    )
    .into()
}

/// Puts the Basic challenge on a 401 from `/login`.
///
/// RFC 9110 asks for a challenge on every 401 of a protected resource, whether
/// the request arrived without credentials or with credentials which were
/// refused. Any other status is handed back untouched, so the challenge never
/// leaks onto a 400 or a 500.
fn with_basic_challenge(mut response: HttpResponse) -> HttpResponse {
    if response.status() == actix_web::http::StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            actix_web::http::header::WWW_AUTHENTICATE,
            actix_web::http::header::HeaderValue::from_static(LOGIN_BASIC_CHALLENGE),
        );
    }
    response
}

/// Turns a missing or malformed Basic `Authorization` header into the 401 that
/// RFC 7617 calls for: the same JSON body as every other failure, plus the
/// challenge which tells the client which scheme to retry with.
fn unauthorized_basic_error(request_id: Uuid) -> Error {
    // The header itself is deliberately not logged: it carries the password.
    event!(Level::ERROR, "Missing or malformed Basic Authorization header");
    InternalError::from_response(
        "Missing or malformed Basic Authorization header",
        with_basic_challenge(HttpResponse::Unauthorized().body(error_body(request_id))),
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
            event!(Level::ERROR, "Incorrect scheme \"{:?}\"", scheme);
        }

        if end.is_some() {
            event!(Level::ERROR, "Extra token after credentials: {:?}", end);
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
                    Some((username, password)) => {
                        Ok(Credentials::new(String::from(username), String::from(password)))
                    }
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
    event!(Level::ERROR, "Unable to read the query string: {err}");
    InternalError::from_response(err, HttpResponse::BadRequest().body(error_body(request_id)))
        .into()
}

// DELETE endpoints

#[delete("/logout")]
pub async fn logout(auth_key: AuthKey) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Logout", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => match crate::core::logout(auth_key.as_str(), &mut connection) {
            Ok(()) => HttpResponse::Ok().finish(),
            Err(e) => e.into_http_response(&format!("{request_id}")),
        },
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

// GET endpoints

#[get("/all_tasks")]
pub async fn get_all_tasks(auth_key: AuthKey) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get All Tasks", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let user = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::get_all_tasks_for_user(&user, &mut connection) {
                Ok(tasks) => HttpResponse::Ok().body(format!(
                    "{}",
                    json!({"request_id": format!("{request_id}"), "tasks": tasks})
                )),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

#[get("/incomplete_tasks")]
pub async fn get_incomplete_tasks(auth_key: AuthKey) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Incomplete Tasks", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let user = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match get_incomplete_tasks_for_user(&user, &mut connection) {
                Ok(tasks) => HttpResponse::Ok().body(format!(
                    "{}",
                    json!({"request_id": format!("{request_id}"), "tasks": tasks})
                )),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

/// The query `GET /task_by_id` reads the task id from.
#[derive(Deserialize)]
struct TaskByIdQuery {
    id: i32,
}

/// Returns a specific task based on the ID of the given task.
///
/// The user must be logged in as the task's owner or have a share on
/// the task in order to get the task.
#[get("/task_by_id")]
pub async fn get_task_by_id(
    auth_key: AuthKey,
    query: actix_web::web::Query<TaskByIdQuery>,
) -> impl Responder {
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
            // the task; anyone else gets the same 401.
            match get_task_permission(&user, &query.id, &mut connection) {
                Ok(Permission::Owner) | Ok(Permission::ReadOnly) | Ok(Permission::ReadWrite) => {
                    match crate::core::get_task_by_id(&query.id, &mut connection) {
                        Ok(task) => HttpResponse::Ok().body(format!(
                            "{}",
                            json!({"request_id": format!("{request_id}"), "task": task})
                        )),
                        Err(e) => e.into_http_response(&format!("{request_id}")),
                    }
                }
                Ok(_) => {
                    event!(
                        Level::ERROR,
                        "Cannot get task {}: user {} has no task access",
                        query.id,
                        user.ref_id()
                    );
                    HttpResponse::Unauthorized().body(error_body(request_id))
                }
                Err(e) => match e {
                    // Don't distinguish "task doesn't exist" from "not your
                    // task": both are the same 401 to avoid leaking which
                    // task IDs exist.
                    ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                        event!(Level::ERROR, "Cannot get task {}: not found", query.id);
                        HttpResponse::Unauthorized().body(error_body(request_id))
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
            HttpResponse::InternalServerError().body(error_body(request_id))
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

/// The query `GET /task_shares` reads the task id from.
#[derive(Deserialize)]
struct TaskSharesQuery {
    id: i32,
}

/// Lists the shares on a task owned by the caller.
#[get("/task_shares")]
pub async fn get_task_shares(
    auth_key: AuthKey,
    query: actix_web::web::Query<TaskSharesQuery>,
) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Task Shares", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match list_shares_for_task(&owner, &query.id, &mut connection) {
                Ok(shares) => {
                    let shares: Vec<serde_json::Value> = shares.iter().map(|s| {
                        json!({"username": s.username(), "permission": s.permission().as_stored_text()})
                    }).collect();
                    HttpResponse::Ok().body(format!(
                        "{}",
                        json!({"request_id": format!("{request_id}"), "shares": shares})
                    ))
                }
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
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
                Ok(user) => HttpResponse::Ok().body(format!(
                    "{}",
                    json!({"request_id": format!("{request_id}"), "username": user.ref_username()})
                )),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
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
            Ok(user) => HttpResponse::Ok().body(format!(
                "{}",
                json!({ "request_id": format!("{}", request_id), "username": user.ref_username()})
            )),
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
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

// POST endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct AddTaskPayload {
    title: String,
    description: Option<String>,
}

/// Creates a new task.
///
/// "Completed" defaults to "false"
#[post("/add_task")]
pub async fn add_task(auth_key: AuthKey, mut payload: actix_web::web::Json<AddTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Add Task", %request_id).entered();

    let title = match TaskTitle::new(std::mem::take(&mut payload.title)) {
        Ok(title) => title,
        Err(e) => {
            event!(Level::ERROR, "{e:?}");
            return HttpResponse::BadRequest().body(error_body_with_message(request_id, e));
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
                Ok(task) => HttpResponse::Created().body(format!(
                    "{}",
                    json!({"request_id": format!("{request_id}"), "task_id": task.id()})
                )),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}
/// Gives a new auth key to the user.
///
/// The credentials are read from an HTTP Basic `Authorization` header, so a
/// request has no body. Every 401 answer, whether the header was missing or
/// malformed or the credentials were refused, carries the Basic challenge that
/// `WWW-Authenticate` requires.
#[post("/login")]
pub async fn login(credentials: Credentials) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Login", %request_id).entered();

    let (raw_username, pw) = credentials.into_parts();

    let un = match Username::new(raw_username) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(error_body_with_message(request_id, message));
        }
    };

    match establish_connection() {
        Ok(mut conn) => {
            let user = match get_user_by_name(&un, &mut conn) {
                Ok(user) => user,
                Err(e) => {
                    return with_basic_challenge(e.into_http_response(&format!("{request_id}")));
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
                    return HttpResponse::InternalServerError().body(error_body(request_id));
                }
            };

            match get_new_auth_key(&user, &pw, &hashed_password, &mut conn) {
                Ok(auth_key) => HttpResponse::Ok().body(format!("{}", json!({ "request_id": format!("{request_id}"), "auth_key": format!("{auth_key}")}))),
                Err(e) => with_basic_challenge(e.into_http_response(&format!("{request_id}"))),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
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
            return HttpResponse::BadRequest().body(error_body(request_id));
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
            return HttpResponse::InternalServerError().body(error_body(request_id));
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
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

// Share management endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct ShareTaskPayload {
    id: i32,
    username: String,
    permission: String,
}

/// Grants another user access to a task owned by the caller, or replaces
/// the permission on an existing share.
#[post("/share_task")]
pub async fn share_task(auth_key: AuthKey, mut payload: actix_web::web::Json<ShareTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Share Task", %request_id).entered();

    let permission = match SharePermission::from_stored_text(&payload.permission) {
        Ok(p) => p,
        Err(e) => {
            event!(Level::ERROR, "Invalid permission: {e:?}");
            return HttpResponse::BadRequest().body(error_body_with_message(
                request_id,
                "permission must be \"read\" or \"read_write\"",
            ));
        }
    };

    let target = match Username::new(std::mem::take(&mut payload.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(error_body_with_message(request_id, message));
        }
    };

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match add_share_for_task(&owner, &payload.id, &target, permission, &mut connection) {
                Ok(()) => HttpResponse::Ok().body(error_body(request_id)),
                // Unauthorized covers both "task doesn't exist" and "not
                // your task", so the 401 doesn't reveal which.
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

// PATCH endpoints

/// The fields a client may change on an existing task.
///
/// `Default` + `PartialEq` let us check for "no changes requested" without
/// enumerating fields: adding a new optional field here automatically
/// extends that check.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
struct TaskUpdates {
    completed: Option<bool>,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct UpdateTaskPayload {
    id: i32,
    #[serde(flatten)]
    updates: TaskUpdates,
}

/// Changes one or more of the task's fields.
///
/// It is an error for all of the optional values to be None; at least on field must be updated for
/// this endpoint to succeed.
#[patch("/task_by_id")]
pub async fn update_task_by_id(
    auth_key: AuthKey,
    mut payload: actix_web::web::Json<UpdateTaskPayload>,
) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Update Task", %request_id).entered();

    // Reject no-op updates without enumerating individual fields: if no
    // optional field is set, this equals the default (all None).
    if payload.updates == TaskUpdates::default() {
        event!(Level::ERROR, "No fields provided to update");
        return HttpResponse::BadRequest().body(error_body_with_message(
            request_id,
            "At least one field must be provided",
        ));
    }

    let title = if let Some(title) = payload.updates.title.as_mut() {
        match TaskTitle::new(std::mem::take(title)) {
            Ok(title) => Some(title),
            Err(e) => {
                event!(Level::ERROR, "{e:?}");
                return HttpResponse::BadRequest().body(error_body_with_message(request_id, e));
            }
        }
    } else {
        None
    };

    let description = payload
        .updates
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
                &payload.id,
                payload.updates.completed,
                title,
                description,
                &mut connection,
            ) {
                Ok(()) => HttpResponse::Ok().body(error_body(request_id)),
                Err(e) => match e {
                    // Don't distinguish "task doesn't exist" from "not your
                    // task": both are the same 401 to avoid leaking which
                    // task IDs exist.
                    ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                        event!(Level::ERROR, "Cannot update task {}: not found", payload.id);
                        HttpResponse::Unauthorized().body(error_body(request_id))
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
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct UnshareTaskPayload {
    id: i32,
    username: String,
}

// DELETE endpoints

/// Removes another user's access to a task owned by the caller.
#[delete("/unshare_task")]
pub async fn unshare_task(auth_key: AuthKey, mut payload: actix_web::web::Json<UnshareTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Unshare Task", %request_id).entered();

    let target = match Username::new(std::mem::take(&mut payload.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(error_body_with_message(request_id, message));
        }
    };

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(auth_key.as_str(), &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match remove_share(&owner, &payload.id, &target, &mut connection) {
                Ok(()) => HttpResponse::Ok().body(error_body(request_id)),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(
                Level::ERROR,
                "Unable to establish connection to database: {e:?}"
            );
            HttpResponse::InternalServerError().body(error_body(request_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{build_app, core::Task};
    use actix_http::Request;
    use actix_web::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
    use actix_web::{App, body::MessageBody, dev::Service, dev::ServiceResponse, test, web};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ctor::ctor;
    use serde::Deserialize;
    use tracing_subscriber::{EnvFilter, fmt};

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

    /// Asserts that a 401 carries the challenge a Basic client needs in order
    /// to know what to retry with.
    fn assert_basic_challenge(response: &ServiceResponse, message: &str) {
        assert_eq!(
            response
                .headers()
                .get(WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some(super::LOGIN_BASIC_CHALLENGE),
            "{message}"
        );
    }

    async fn register_and_login<E: std::fmt::Debug>(
        app: impl Service<Request, Response = ServiceResponse, Error = E>,
        username: &str,
        password: &str,
    ) -> String {
        assert_response_success(
            register(&app, username, password).await,
            "Unable to register account",
        );
        let login_response =
            assert_response_success(login(&app, username, password).await, "Could not login.");
        extract_json_string(login_response, "auth_key")
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

    async fn login<E: std::fmt::Debug>(
        app: impl Service<Request, Response = ServiceResponse, Error = E>,
        username: &str,
        password: &str,
    ) -> ServiceResponse {
        let credentials = STANDARD.encode(format!("{username}:{password}"));
        let request = test::TestRequest::post()
            .uri("/login")
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
            .uri("/add_task")
            .set_json(super::AddTaskPayload {
                title: String::from(title),
                description: description.map(String::from),
            })
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

    fn extract_json_i32(response: ServiceResponse, key: &str) -> i32 {
        extract_json_from_constructor(response, key, |v| match v.as_i64() {
            Some(i) => i as i32,
            None => panic!("{} is not a number! {:?}", key, v),
        })
    }

    fn extract_json_string(response: ServiceResponse, key: &str) -> String {
        extract_json_from_constructor(response, key, |v| match v.as_str() {
            Some(s) => String::from(s),
            None => panic!("{} is not a string! {:?}", key, v),
        })
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

        let auth_key = register_and_login(&app, username.as_str(), password.as_str()).await;

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

        let response = login(&app, username, "not-the-correct-password").await;
        assert_client_error(response, "Response did not indicate client error");
    }

    #[actix_web::test]
    async fn login_requires_basic_authorization_header() {
        let app = test::init_service(build_app!()).await;

        // No Authorization header at all.
        let request = test::TestRequest::post().uri("/login").to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "Login without an Authorization header did not return 401."
        );
        assert_basic_challenge(&response, "A missing header got no challenge.");

        // An auth key is not a credential pair, so the wrong scheme is refused.
        let request = test::TestRequest::post()
            .uri("/login")
            .insert_header((AUTHORIZATION, "Bearer some-auth-key"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "Login with a Bearer header did not return 401."
        );
        assert_basic_challenge(&response, "A wrong scheme got no challenge.");

        // A token which is not base64.
        let request = test::TestRequest::post()
            .uri("/login")
            .insert_header((AUTHORIZATION, "Basic not-valid-base64!"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "Login with an undecodable token did not return 401."
        );
        assert_basic_challenge(&response, "An undecodable token got no challenge.");

        // Valid base64, but the decoded text has no colon to split on.
        let request = test::TestRequest::post()
            .uri("/login")
            .insert_header((
                AUTHORIZATION,
                format!("Basic {}", STANDARD.encode("no-colon")),
            ))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "Login with a token missing a colon did not return 401."
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
        let response = login(&app, username, "not-the-correct-password").await;
        assert_eq!(
            response.status().as_u16(),
            401,
            "Login with a wrong password did not return 401."
        );
        assert_basic_challenge(&response, "Refused credentials got no challenge.");

        // An unknown username is *not* answered the same way: it gives a 404
        // rather than a 401. A 404 needs no challenge, so none is sent.
        let response = login(&app, "no-such-username", password).await;
        assert_eq!(
            response.status().as_u16(),
            404,
            "Login for an unknown user did not return 404."
        );
        assert!(
            response.headers().get(WWW_AUTHENTICATE).is_none(),
            "A 404 must not carry the Basic challenge, which belongs to 401s."
        );
    }

    #[actix_web::test]
    async fn login_password_may_contain_a_colon() {
        let username = "colon-password-username";
        let password = "colon:password:with:colons";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let whoami_response = assert_response_success(
            test::call_service(&app, whoami_request).await,
            "Whoami request failed.",
        );
        assert_eq!(
            extract_json_string(whoami_response, "username"),
            username
        );
    }

    #[actix_web::test]
    async fn can_logout() {
        let username = "can-logout-username";
        let password = "can-logout-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        let logout_request = test::TestRequest::delete()
            .uri("/logout")
            .insert_header((AUTHORIZATION, format!("bearer {auth_key}")))
            .to_request();
        assert_response_success(
            test::call_service(&app, logout_request).await,
            "Unable to log out.",
        );

        let whoami_request = test::TestRequest::get()
            .uri("/whoami")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let whoami_response = test::call_service(&app, whoami_request).await;
        assert_eq!(
            whoami_response.status().as_u16(),
            401,
            "WhoAmI did not return client error."
        );
    }

    #[actix_web::test]
    async fn can_add_task() {
        let username = String::from("can-add-task-username");
        let password = String::from("can-add-task-password");
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username.as_str(), password.as_str()).await;

        assert_response_success(
            add_task(&app, &auth_key, "test title", Some("test description")).await,
            "Unable to add task.",
        );
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
        assert_client_error(response, "Response did not indicate client failure");
    }

    #[actix_web::test]
    async fn can_get_task() {
        let username = String::from("can-get-task-username");
        let password = String::from("can-get-task-password");
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username.as_str(), password.as_str()).await;

        let add_task_response = assert_response_success(
            add_task(&app, &auth_key, "test title", Some("test description")).await,
            "Unable to add task.",
        );

        let task_id = extract_json_i32(add_task_response, "task_id");
        let get_task_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
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

        let owner_auth_key = register_and_login(&app, owner_username, owner_password).await;

        let add_task_response = assert_response_success(
            add_task(
                &app,
                &owner_auth_key,
                "test title",
                Some("test description"),
            )
            .await,
            "Unable to add task.",
        );
        let task_id = extract_json_i32(add_task_response, "task_id");

        let non_owner_auth_key =
            register_and_login(&app, non_owner_username, non_owner_password).await;

        let get_task_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {non_owner_auth_key}")))
            .to_request();
        let get_task_response = test::call_service(&app, get_task_request).await;
        assert!(
            get_task_response.status().is_client_error(),
            "Getting task did not indicate client error."
        );
    }

    #[actix_web::test]
    async fn cannot_get_nonexistent_task() {
        let username = "cannot-get-nonexistent-task-username";
        let password = "cannot-get-nonexistent-task-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        let get_task_request = test::TestRequest::get()
            .uri("/task_by_id?id=-1")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_task_response = test::call_service(&app, get_task_request).await;
        // A missing task must look the same as a task owned by someone else: 401, not 404.
        assert_eq!(
            get_task_response.status().as_u16(),
            401,
            "Getting a nonexistent task should not reveal its nonexistence."
        );
    }

    #[actix_web::test]
    async fn cannot_get_task_without_a_valid_id_query_parameter() {
        let username = "cannot-get-task-without-id-username";
        let password = "cannot-get-task-without-id-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        for uri in [
            "/task_by_id",
            "/task_by_id?id=",
            "/task_by_id?id=not-a-number",
        ] {
            let get_task_request = test::TestRequest::get()
                .uri(uri)
                .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
                .to_request();
            let get_task_response = test::call_service(&app, get_task_request).await;
            assert_eq!(
                get_task_response.status().as_u16(),
                400,
                "{uri} should be a 400."
            );
            assert_json_error_with_request_id(get_task_response);
        }
    }

    #[actix_web::test]
    async fn cannot_update_nonexistent_task() {
        let username = "cannot-update-nonexistent-task-username";
        let password = "cannot-update-nonexistent-task-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: -1,
                updates: super::TaskUpdates {
                    completed: Some(true),
                    title: None,
                    description: None,
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(
            update_response.status().as_u16(),
            401,
            "Updating a nonexistent task should not reveal its nonexistence."
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

        let auth_key = register_and_login(&app, username, password).await;

        let add_task_response = assert_response_success(
            add_task(&app, &auth_key, "test title", Some("test description")).await,
            "Unable to add task.",
        );
        let task_id = extract_json_i32(add_task_response, "task_id");

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: task_id,
                updates: super::TaskUpdates {
                    completed,
                    title,
                    description,
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "Unable to update task.",
        );

        let get_task_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
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

        let auth_key = register_and_login(&app, username, password).await;

        let first_add_response = assert_response_success(
            add_task(&app, &auth_key, "first task", None).await,
            "Unable to add first task.",
        );
        let first_task_id = extract_json_i32(first_add_response, "task_id");

        let second_add_response = assert_response_success(
            add_task(&app, &auth_key, "second task", None).await,
            "Unable to add second task.",
        );
        let second_task_id = extract_json_i32(second_add_response, "task_id");

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: first_task_id,
                updates: super::TaskUpdates {
                    completed: Some(true),
                    title: None,
                    description: None,
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {}", auth_key.clone())))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "Unable to update first task.",
        );

        let get_second = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={second_task_id}"))
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

    #[actix_web::test]
    async fn can_get_all_tasks() {
        let username = "can-get-all-tasks-username";
        let password = "can-get-all-tasks-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        assert_response_success(
            add_task(&app, &auth_key, "task 1", None).await,
            "Unable to add first task.",
        );

        assert_response_success(
            add_task(&app, &auth_key, "task 2", None).await,
            "Unable to add second task.",
        );

        let get_tasks_request = test::TestRequest::get()
            .uri("/all_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_tasks_response = assert_response_success(
            test::call_service(&app, get_tasks_request).await,
            "Could not get tasks!",
        );
        let tasks: Vec<Task> = extract_json_tasks(get_tasks_response);
        assert_eq!(tasks.len(), 2, "Expected 2 tasks, found {}", tasks.len());
    }

    #[actix_web::test]
    async fn can_get_incomplete_tasks() {
        let username = "can-get-incomplete-tasks-username";
        let password = "can-get-incomplete-tasks-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        assert_response_success(
            add_task(&app, &auth_key, "task 1", None).await,
            "Unable to add first task.",
        );

        let second_add_task_response = assert_response_success(
            add_task(&app, &auth_key, "task 2", None).await,
            "Unable to add second task.",
        );
        let second_task_id = extract_json_i32(second_add_task_response, "task_id");

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: second_task_id,
                updates: super::TaskUpdates {
                    completed: Some(true),
                    title: None,
                    description: None,
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {}", auth_key.clone())))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "Unable to mark second task completed.",
        );

        let get_tasks_request = test::TestRequest::get()
            .uri("/incomplete_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        let get_tasks_response = assert_response_success(
            test::call_service(&app, get_tasks_request).await,
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

    async fn share_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
        username: &str,
        permission: &str,
    ) -> ServiceResponse {
        let request = test::TestRequest::post()
            .uri("/share_task")
            .set_json(super::ShareTaskPayload {
                id,
                username: String::from(username),
                permission: String::from(permission),
            })
            .insert_header((AUTHORIZATION, format!("Bearer {auth_key}")))
            .to_request();
        test::call_service(app, request).await
    }

    async fn unshare_task(
        app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>,
        auth_key: &str,
        id: i32,
        username: &str,
    ) -> ServiceResponse {
        let request = test::TestRequest::delete()
            .uri("/unshare_task")
            .set_json(super::UnshareTaskPayload {
                id,
                username: String::from(username),
            })
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
            .uri(&format!("/task_shares?id={id}"))
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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let add_response = assert_response_success(
            add_task(&app, &owner_key, "shared task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let add_response = assert_response_success(
            add_task(&app, &owner_key, "shared task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );
        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Re-share request failed",
        );

        let list_response = assert_response_success(
            get_task_shares(&app, &owner_key, task_id).await,
            "list shares",
        );
        let shares = extract_shares(list_response);
        assert_eq!(shares.len(), 1, "Re-sharing should replace, not duplicate");
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read_write");
    }

    #[actix_web::test]
    async fn non_owner_cannot_share_unshare_or_list() {
        let owner_username = "nonowner-share-owner-username";
        let non_owner_username = "nonowner-share-nonowner-username";
        let password = "nonowner-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_login(&app, owner_username, password).await;
        let non_owner_key = register_and_login(&app, non_owner_username, password).await;

        let add_response =
            assert_response_success(add_task(&app, &owner_key, "task", None).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_eq!(
            share_task(&app, &non_owner_key, task_id, non_owner_username, "read")
                .await
                .status()
                .as_u16(),
            401
        );
        assert_eq!(
            unshare_task(&app, &non_owner_key, task_id, non_owner_username)
                .await
                .status()
                .as_u16(),
            401
        );
        assert_eq!(
            get_task_shares(&app, &non_owner_key, task_id)
                .await
                .status()
                .as_u16(),
            401
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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let add_response =
            assert_response_success(add_task(&app, &owner_key, "task", None).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

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

    #[actix_web::test]
    async fn unknown_target_username_gives_404() {
        let owner_username = "unknown-target-owner-username";
        let password = "unknown-target-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_login(&app, owner_username, password).await;
        let add_response =
            assert_response_success(add_task(&app, &owner_key, "task", None).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

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
    async fn missing_task_returns_same_401_as_not_owned() {
        let owner_username = "oracle-owner-username";
        let other_username = "oracle-other-username";
        let password = "oracle-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_login(&app, owner_username, password).await;
        let other_key = register_and_login(&app, other_username, password).await;

        let add_response =
            assert_response_success(add_task(&app, &owner_key, "task", None).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        // Non-owner trying to manage an existing task
        let not_owned = share_task(&app, &other_key, task_id, other_username, "read").await;
        // Anyone trying to manage a missing task
        let missing = share_task(&app, &owner_key, -1, other_username, "read").await;
        assert_eq!(not_owned.status().as_u16(), 401);
        assert_eq!(missing.status().as_u16(), 401);

        let not_owned_list = get_task_shares(&app, &other_key, task_id).await;
        let missing_list = get_task_shares(&app, &owner_key, -1).await;
        assert_eq!(not_owned_list.status().as_u16(), 401);
        assert_eq!(missing_list.status().as_u16(), 401);

        let not_owned_unshare = unshare_task(&app, &other_key, task_id, other_username).await;
        let missing_unshare = unshare_task(&app, &owner_key, -1, other_username).await;
        assert_eq!(not_owned_unshare.status().as_u16(), 401);
        assert_eq!(missing_unshare.status().as_u16(), 401);
    }

    #[actix_web::test]
    async fn read_write_sharee_can_update_task_fields() {
        let owner_username = "rw-update-owner-username";
        let grantee_username = "rw-update-grantee-username";
        let password = "rw-update-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_login(&app, owner_username, password).await;
        let grantee_key = register_and_login(&app, grantee_username, password).await;

        let add_response = assert_response_success(
            add_task(
                &app,
                &owner_key,
                "shared task",
                Some("original description"),
            )
            .await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: task_id,
                updates: super::TaskUpdates {
                    completed: Some(true),
                    title: Some(String::from("updated by grantee")),
                    description: Some(String::from("updated description")),
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {}", grantee_key.clone())))
            .to_request();
        assert_response_success(
            test::call_service(&app, update_request).await,
            "read_write grantee should be able to update the task",
        );

        let get_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let grantee_key = register_and_login(&app, grantee_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "shared task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: task_id,
                updates: super::TaskUpdates {
                    completed: Some(true),
                    title: None,
                    description: None,
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(
            update_response.status().as_u16(),
            401,
            "read-only sharee must not update the task"
        );

        // The task itself must be unchanged.
        let get_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let stranger_key = register_and_login(&app, stranger_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "private task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        let update_request = test::TestRequest::patch()
            .uri("/task_by_id")
            .set_json(super::UpdateTaskPayload {
                id: task_id,
                updates: super::TaskUpdates {
                    completed: Some(true),
                    title: None,
                    description: None,
                },
            })
            .insert_header((AUTHORIZATION, format!("Bearer {stranger_key}")))
            .to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(
            update_response.status().as_u16(),
            401,
            "Unrelated user must not update the task"
        );

        // The task itself must be unchanged.
        let get_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let grantee_key = register_and_login(&app, grantee_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "shared task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        let failed_share =
            share_task(&app, &grantee_key, task_id, owner_username, "read_write").await;
        assert_eq!(failed_share.status().as_u16(), 401);

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
        assert_eq!(failed_unshare.status().as_u16(), 401);

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

        assert_eq!(
            get_task_shares(&app, &grantee_key, task_id)
                .await
                .status()
                .as_u16(),
            401
        );
    }

    #[actix_web::test]
    async fn cannot_list_shares_without_a_valid_id_query_parameter() {
        let username = "cannot-list-shares-without-id-username";
        let password = "cannot-list-shares-without-id-password";
        let app = test::init_service(build_app!()).await;

        let auth_key = register_and_login(&app, username, password).await;

        for uri in [
            "/task_shares",
            "/task_shares?id=",
            "/task_shares?id=not-a-number",
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
    async fn read_share_grants_get_and_listing_access() {
        let owner_username = "read-share-owner-username";
        let grantee_username = "read-share-grantee-username";
        let password = "read-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_login(&app, owner_username, password).await;
        let grantee_key = register_and_login(&app, grantee_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "shared task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read").await,
            "Share request failed",
        );

        // The grantee can now fetch the task directly...
        let get_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
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
        let list_request = test::TestRequest::get()
            .uri("/all_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        let list_response = assert_response_success(
            test::call_service(&app, list_request).await,
            "list grantee tasks",
        );
        let tasks: Vec<Task> = extract_json_tasks(list_response);
        assert_eq!(tasks.len(), 1);
        assert_eq!(*tasks[0].id(), task_id);

        let incomplete_request = test::TestRequest::get()
            .uri("/incomplete_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        let incomplete_response = assert_response_success(
            test::call_service(&app, incomplete_request).await,
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

        let owner_key = register_and_login(&app, owner_username, password).await;
        let grantee_key = register_and_login(&app, grantee_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "shared task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(
            share_task(&app, &owner_key, task_id, grantee_username, "read_write").await,
            "Share request failed",
        );

        let get_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        assert_response_success(
            test::call_service(&app, get_request).await,
            "read_write grantee should be able to get the task",
        );

        let list_request = test::TestRequest::get()
            .uri("/all_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {grantee_key}")))
            .to_request();
        let list_response = assert_response_success(
            test::call_service(&app, list_request).await,
            "list grantee tasks",
        );
        let tasks: Vec<Task> = extract_json_tasks(list_response);
        assert_eq!(tasks.len(), 1);
        assert_eq!(*tasks[0].id(), task_id);
    }

    #[actix_web::test]
    async fn no_share_means_401_and_no_listing_entry() {
        let owner_username = "no-share-owner-username";
        let stranger_username = "no-share-stranger-username";
        let password = "no-share-password";
        let app = test::init_service(build_app!()).await;

        let owner_key = register_and_login(&app, owner_username, password).await;
        let stranger_key = register_and_login(&app, stranger_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "private task", None).await,
            "add task",
        );
        let task_id = extract_json_i32(add_response, "task_id");

        let get_request = test::TestRequest::get()
            .uri(&format!("/task_by_id?id={task_id}"))
            .insert_header((AUTHORIZATION, format!("Bearer {stranger_key}")))
            .to_request();
        let get_response = test::call_service(&app, get_request).await;
        assert_eq!(get_response.status().as_u16(), 401);

        let list_request = test::TestRequest::get()
            .uri("/all_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {stranger_key}")))
            .to_request();
        let list_response = assert_response_success(
            test::call_service(&app, list_request).await,
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

        let owner_key = register_and_login(&app, owner_username, password).await;

        let add_response = assert_response_success(
            add_task(&app, &owner_key, "owned task", None).await,
            "add task",
        );
        let owned_task_id = extract_json_i32(add_response, "task_id");

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

        let list_request = test::TestRequest::get()
            .uri("/all_tasks")
            .insert_header((AUTHORIZATION, format!("Bearer {owner_key}")))
            .to_request();
        let list_response = assert_response_success(
            test::call_service(&app, list_request).await,
            "list owner tasks",
        );
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

        let auth_key = register_and_login(&app, username, password).await;

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

        let auth_key = register_and_login(&app, username, password).await;

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

        let auth_key = register_and_login(&app, username, password).await;

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

    // Shared extractor tests

    /// The query the endpoints read their task or user id from.
    #[derive(Deserialize)]
    struct IdQuery {
        id: i32,
    }
}
