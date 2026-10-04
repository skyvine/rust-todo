use crate::core::{
    add_share_for_task,
    add_user,
    auth_key_to_user,
    establish_connection,
    get_incomplete_tasks_for_user,
    get_new_auth_key,
    get_task_permission,
    get_user_by_name,
    list_shares_for_task,
    remove_share,
    ApplicationError,
    Permission,
    SharePermission,
};
use crate::domain_types::{CleartextPassword, TaskDescription, TaskTitle, Username};

use actix_web::{delete, get, post, HttpResponse, Responder};
use argon2::{
    password_hash::{
        rand_core::OsRng, PasswordHash, PasswordHasher, SaltString
    },
    Argon2
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{event, span, Level};
use uuid::Uuid;
use zeroize::{ZeroizeOnDrop};

impl ApplicationError {
    fn into_http_response(self, request_id: &String) -> HttpResponse {
        match self {
            ApplicationError::DieselError(e) => {
                event!(Level::ERROR, "Diesel error: {e}");
                match e {
                    diesel::result::Error::NotFound => HttpResponse::NotFound().body(format!("{}", json!({"request_id": request_id}))),
                    _ => HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": request_id}))),
                }
            },
            ApplicationError::InvalidAuthKey => {
                event!(Level::ERROR, "Invalid auth key");
                HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": request_id})))
            }
            ApplicationError::InvalidData(message) => {
                event!(Level::ERROR, "Invalid data: {message}");
                HttpResponse::BadRequest().body(format!("{}", json!({"request_id": request_id, "message": message})))
            }
            ApplicationError::InvalidPassword => {
                event!(Level::ERROR, "Invalid password");
                HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": request_id})))
            },
            ApplicationError::QueryFailed(e) => {
                event!(Level::ERROR, "Query failed: {e}");
                HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": request_id})))
            },
            ApplicationError::Unauthorized => {
                event!(Level::ERROR, "Unauthorized");
                HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": request_id})))
            }
            ApplicationError::UserExists => HttpResponse::Conflict().body(format!("{}", json!({"request_id": request_id}))),
        }
    }
}

// DELETE endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct LogoutPayload {
    auth_key: String,
}

#[delete("/logout")]
pub async fn logout(payload: actix_web::web::Json<LogoutPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Logout", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            match crate::core::logout(&payload.auth_key, &mut connection) {
                Ok(()) => HttpResponse::Ok().finish(),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

// GET endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct GetAllTasksPayload {
    auth_key: String
}

#[get("/all_tasks")]
pub async fn get_all_tasks(payload: actix_web::web::Json<GetAllTasksPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get All Tasks", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let user = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::get_all_tasks_for_user(&user, &mut connection) {
                Ok(tasks) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}"), "tasks": tasks}))),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct GetIncompleteTasksPayload {
    auth_key: String
}

#[get("/incomplete_tasks")]
pub async fn get_incomplete_tasks(payload: actix_web::web::Json<GetIncompleteTasksPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Incomplete Tasks", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let user = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match get_incomplete_tasks_for_user(&user, &mut connection) {
                Ok(tasks) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}"), "tasks": tasks}))),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct GetTaskByIdPayload {
    auth_key: String,
    id: i32,
}

/// Returns a specific task based on the ID of the given task.
/// 
/// The user must be logged in as the task's owner or have a share on
/// the task in order to get the task.
#[get("/task_by_id")]
pub async fn get_task_by_id(payload: actix_web::web::Json<GetTaskByIdPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Task by ID", %request_id).entered();


    match establish_connection() {
        Ok(mut connection) => {
            // Authenticate before fetching anything
            let user = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            // Owner, read-only, and read-write share holders may all view
            // the task; anyone else gets the same 401.
            match get_task_permission(&user, &payload.id, &mut connection) {
                Ok(Permission::Owner) | Ok(Permission::ReadOnly) | Ok(Permission::ReadWrite) => {
                    match crate::core::get_task_by_id(&payload.id, &mut connection) {
                        Ok(task) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}"), "task": task}))),
                        Err(e) => e.into_http_response(&format!("{request_id}")),
                    }
                },
                Ok(_) => {
                    event!(Level::ERROR, "Cannot get task {}: user {} has no task access", payload.id, user.ref_id());
                    HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                },
                Err(e) => match e {
                    // Don't distinguish "task doesn't exist" from "not your
                    // task": both are the same 401 to avoid leaking which
                    // task IDs exist.
                    ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                        event!(Level::ERROR, "Cannot get task {}: not found", payload.id);
                        HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                    },
                    other => other.into_http_response(&format!("{request_id}")),
                },
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
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

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct GetUserByIdPayload {
    auth_key: String,
    id: i32,
}

/// Resolves a user id (as exposed in `Task`'s `owner` field and in share
/// responses) to that user's username. User ids are public, so an unknown
/// id simply returns 404.
#[get("/user_by_id")]
pub async fn get_user_by_id(payload: actix_web::web::Json<GetUserByIdPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get User By ID", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let _user = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::get_user_by_id(&payload.id, &mut connection) {
                Ok(user) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}"), "username": user.ref_username()}))),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct WhoAmIPayload {
    auth_key: String
}

/// Returns the username of the currently logged in user.
#[get("/whoami")]
pub async fn whoami(payload: actix_web::web::Json<WhoAmIPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Who Am I Request", %request_id).entered();

    match establish_connection() {
        Ok(mut conn) => 
            match auth_key_to_user(&payload.auth_key, &mut conn) {
                Ok(user) => HttpResponse::Ok().body(format!("{}", json!({ "request_id": format!("{}", request_id), "username": user.ref_username()}))),
                Err(e) => {
                    event!(Level::ERROR, "Unable to look up user");
                    e.into_http_response(&format!("{request_id}"))
                }
            },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{}", request_id)})))
        }
    }
}

// POST endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct AddTaskPayload {
    auth_key:    String,
    title:       String,
    description: Option<String>,
}

/// Creates a new task.
/// 
/// "Completed" defaults to "false"
#[post("/add_task")]
pub async fn add_task(mut payload: actix_web::web::Json<AddTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Add Task", %request_id).entered();

    let title = match TaskTitle::new(std::mem::take(&mut payload.title)) {
        Ok(title) => title,
        Err(e) => {
            event!(Level::ERROR, "{e:?}");
            return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": e})));
        }
    };
    let description = payload.description.take().map(TaskDescription::new);
    let description = description.as_ref();

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}"))
            };

            match crate::core::add_task(&owner, &title, false, description, &mut connection) {
                Ok(task) => HttpResponse::Created().body(format!("{}", json!({"request_id": format!("{request_id}"), "task_id": task.id()}))),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}
#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct LoginPayload {
    username: String,
    password: String
}

/// Gives a new auth key to the user.
#[post("/login")]
pub async fn login(mut payload: actix_web::web::Json<LoginPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Login", %request_id).entered();

    let un = match Username::new(std::mem::take(&mut payload.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": message})));
        }
    };

    let pw = CleartextPassword::new(std::mem::take(&mut payload.password));

    match establish_connection() {
        Ok(mut conn) => {
            let user = match get_user_by_name(&un, &mut conn) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            let hashed_password = match PasswordHash::new(user.ref_password()) {
                Ok(ph) => ph,
                Err(e) => {
                    event!(Level::ERROR, "Unable to parse hashed password ({}): {e:?}", user.ref_password());
                    return HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})));
                }
            };

            match get_new_auth_key(&user, &pw, &hashed_password, &mut conn) {
                Ok(auth_key) => HttpResponse::Ok().body(format!("{}", json!({ "request_id": format!("{request_id}"), "auth_key": format!("{auth_key}")}))),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
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
pub async fn register_account(mut account_info: actix_web::web::Json<UserRegistrationPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Registering Account", %request_id).entered();

    // move not allowed in .values() call below, avoid cloning by replacing
    let un = match Username::new(std::mem::take(&mut account_info.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}")})));
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
            return HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})));
        }
    };

    match establish_connection() {
        Ok(mut conn) => {
            event!(Level::TRACE, "Established connection to database.");

            match add_user(&un, &hashed_password, &mut conn) {
                Ok(()) => HttpResponse::Created().finish(),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },

        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}.");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

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
    auth_key: String,
    #[serde(flatten)]
    updates: TaskUpdates,
}

/// Changes one or more of the task's fields.
/// 
/// It is an error for all of the optional values to be None; at least on field must be updated for
/// this endpoint to succeed.
#[post("/task_by_id")]
pub async fn update_task_by_id(mut payload: actix_web::web::Json<UpdateTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Update Task", %request_id).entered();

    // Reject no-op updates without enumerating individual fields: if no
    // optional field is set, this equals the default (all None).
    if payload.updates == TaskUpdates::default() {
        event!(Level::ERROR, "No fields provided to update");
        return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": "At least one field must be provided"})));
    }

    let title = if let Some(title) = payload.updates.title.as_mut() {
        match TaskTitle::new(std::mem::take(title)) {
            Ok(title) => Some(title),
            Err(e) => {
                event!(Level::ERROR, "{e:?}");
                return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": e})));
            }
        }
    } else {
        None
    };

    let description = payload.updates.description.as_mut().map(|d| TaskDescription::new(std::mem::take(d)));

    match establish_connection() {
        Ok(mut connection) => {
            let caller = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::update_task(&caller, &payload.id, payload.updates.completed, title, description, &mut connection) {
                Ok(()) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}")}))),
                Err(e) => match e {
                    // Don't distinguish "task doesn't exist" from "not your
                    // task": both are the same 401 to avoid leaking which
                    // task IDs exist.
                    ApplicationError::DieselError(diesel::result::Error::NotFound) => {
                        event!(Level::ERROR, "Cannot update task {}: not found", payload.id);
                        HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                    },
                    other => other.into_http_response(&format!("{request_id}")),
                },
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

// Share management endpoints

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct ShareTaskPayload {
    auth_key: String,
    id: i32,
    username: String,
    permission: String,
}

/// Grants another user access to a task owned by the caller, or replaces
/// the permission on an existing share.
#[post("/share_task")]
pub async fn share_task(mut payload: actix_web::web::Json<ShareTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Share Task", %request_id).entered();

    let permission = match SharePermission::from_stored_text(&payload.permission) {
        Ok(p) => p,
        Err(e) => {
            event!(Level::ERROR, "Invalid permission: {e:?}");
            return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": "permission must be \"read\" or \"read_write\""})));
        }
    };

    let target = match Username::new(std::mem::take(&mut payload.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": message})));
        }
    };

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match add_share_for_task(&owner, &payload.id, &target, permission, &mut connection) {
                Ok(()) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}")}))),
                // Unauthorized covers both "task doesn't exist" and "not
                // your task", so the 401 doesn't reveal which.
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct UnshareTaskPayload {
    auth_key: String,
    id: i32,
    username: String,
}

/// Removes another user's access to a task owned by the caller.
#[delete("/unshare_task")]
pub async fn unshare_task(mut payload: actix_web::web::Json<UnshareTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Unshare Task", %request_id).entered();

    let target = match Username::new(std::mem::take(&mut payload.username)) {
        Ok(un) => un,
        Err(message) => {
            event!(Level::ERROR, "{message}");
            return HttpResponse::BadRequest().body(format!("{}", json!({"request_id": format!("{request_id}"), "message": message})));
        }
    };

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match remove_share(&owner, &payload.id, &target, &mut connection) {
                Ok(()) => HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}")}))),
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct GetTaskSharesPayload {
    auth_key: String,
    id: i32,
}

/// Lists the shares on a task owned by the caller.
#[get("/task_shares")]
pub async fn get_task_shares(payload: actix_web::web::Json<GetTaskSharesPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Task Shares", %request_id).entered();

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match list_shares_for_task(&owner, &payload.id, &mut connection) {
                Ok(shares) => {
                    let shares: Vec<serde_json::Value> = shares.iter().map(|s| {
                        json!({"username": s.username(), "permission": s.permission().as_stored_text()})
                    }).collect();
                    HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}"), "shares": shares})))
                },
                Err(e) => e.into_http_response(&format!("{request_id}")),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_http::Request;
    use actix_web::{body::MessageBody, dev::Service, dev::ServiceResponse, test, App};
    use crate::{build_app, core::Task};
    use ctor::ctor;
    use tracing_subscriber::{fmt, EnvFilter};

    #[ctor]
    unsafe fn global_init() {
        use crate::schema::auth_keys::dsl::*;
        use crate::schema::tasks::dsl::*;
        use crate::schema::users::dsl::*;
        use diesel::prelude::*;

        // Print log messages to help debug failed tests
        fmt().event_format(fmt::format().pretty()).with_env_filter(EnvFilter::from_default_env()).init();

        tracing::event!(tracing::Level::TRACE, "Deleting all entries in tables.");

        // The tests depend on the state of the database, so ensure we always start with a clean slate.
        match diesel::delete(auth_keys).execute(&mut super::establish_connection().expect("Unable to connect to test database.")) {
            Ok(_) => (),
            Err(e) => panic!("Unable to delete auth_keys entries: {e:?}")
        };

        match diesel::delete(tasks).execute(&mut super::establish_connection().expect("Unable to connect to test database.")) {
            Ok(_) => (),
            Err(e) => panic!("Unable to delete tasks entries: {e:?}")
        };

        match diesel::delete(users).execute(&mut super::establish_connection().expect("Unable to connect to test database.")) {
            Ok(_) => (),
            Err(e) => panic!("Unable to delete users entries: {e:?}")
        };
    }

    fn assert_response_success(response: ServiceResponse, message: &str) -> ServiceResponse{
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

    async fn register<E: std::fmt::Debug>(app: impl Service<Request, Response = ServiceResponse, Error = E>, username: &str, password: &str) -> ServiceResponse {
        let request = test::TestRequest::post().uri("/register_account").set_json(super::UserRegistrationPayload {
            username: String::from(username),
            password: String::from(password)
        }).to_request();
        test::call_service(&app, request).await
    }

    async fn login<E: std::fmt::Debug>(app: impl Service<Request, Response = ServiceResponse, Error = E>, username: &str, password: &str) -> ServiceResponse {
        let request = test::TestRequest::post().uri("/login").set_json(super::LoginPayload {
            username: String::from(username),
            password: String::from(password),
        }).to_request();
        test::call_service(&app, request).await
    }

    fn extract_json_from_constructor<Constructor, Output>(response: ServiceResponse, key: &str, constructor: Constructor) -> Output
        where Constructor: FnOnce(&serde_json::Value) -> Output
    {
        match response.into_body().try_into_bytes() {
            Ok(bytes) =>
                match serde_json::from_slice::<serde_json::Value>(bytes.as_ref()) {
                    Ok(dict) => {
                        constructor(&dict[key])
                    },
                    Err(_) => panic!("Unable to deserialize alleged JSON: {bytes:?}"),
                },
            Err(e) => {
                panic!("Unable to extract bytes from response {e:?}")
            }
        }
    }

    fn extract_json_i32(response: ServiceResponse, key: &str) -> i32 {
        extract_json_from_constructor(response, key, |v|
            match v.as_i64() {
                Some(i) => i as i32,
                None => panic!("{} is not a number! {:?}", key, v),
            })
    }

    fn extract_json_string(response: ServiceResponse, key: &str) -> String {
        extract_json_from_constructor(response, key, |v|
            match v.as_str() {
                Some(s) => String::from(s),
                None => panic!("{} is not a string! {:?}", key, v),
            })
    }

    #[actix_web::test]
    async fn is_alive() {
        let app = test::init_service(build_app!()).await;
        let request = test::TestRequest::get().uri("/is_alive").to_request();
        let response = assert_response_success(test::call_service(&app, request).await, "Is alive check failed.");
        assert_eq!(response.into_body().size(), actix_web::body::BodySize::Sized(0));
    }

    #[actix_web::test]
    async fn regitering_account_is_successful() {
        let app = test::init_service(build_app!()).await;
        let response = register(app, "register-account-is-successful-name", "register-account-is-successful-password").await;
        assert_response_success(response, "Error status code");
    }

    #[actix_web::test]
    async fn duplicate_username_fails() {
        let name = String::from("duplicated-name");
        let password = String::from("duplicated-password");
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, name.as_str(), password.as_str()).await, "Failed to register the account once.");

        let response = register(&app, name.as_str(), password.as_str()).await;
        assert_client_error(response, "Response did not indicate client failure");
    }

    #[actix_web::test]
    async fn auth_key_is_recognized() {
        let username = String::from("auth-key-registered-name");
        let password = String::from("auth-key-registered-password");
        let app =
            test::init_service(build_app!()).await;

        assert_response_success(register(&app, username.as_str(), password.as_str()).await, "Unable to register account");

        let login_response = assert_response_success(login(&app, username.as_str(), password.as_str()).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let whoami_request = test::TestRequest::get().uri("/whoami").set_json(super::WhoAmIPayload {
            auth_key,
        }).to_request();
        let whoami_response = assert_response_success(test::call_service(&app, whoami_request).await, "Whoami request failed.");

        let response_name = extract_json_string(whoami_response, "username");
        assert_eq!(username, response_name);
    }

    #[actix_web::test]
    async fn invalid_password_fails() {
        let username = "invalid-password-username";
        let password = "invalid-password-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account");

        let response = login(&app, username, "not-the-correct-password").await;
        assert_client_error(response, "Response did not indicate client error");
    }

    #[actix_web::test]
    async fn can_logout() {
        let username = "can-logout-username";
        let password = "can-logout-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account");

        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");
        
        let logout_request = test::TestRequest::delete().uri("/logout").set_json(super::LogoutPayload { auth_key: auth_key.clone() }).to_request();
        assert_response_success(test::call_service(&app, logout_request).await, "Unable to log out.");

        let whoami_request = test::TestRequest::get().uri("/whoami").set_json(super::WhoAmIPayload { auth_key }).to_request();
        let whoami_response = test::call_service(&app, whoami_request).await;
        assert_eq!(whoami_response.status().as_u16(), 401, "WhoAmI did not return client error.");
    }

    #[actix_web::test]
    async fn can_add_task() {
        let username = String::from("can-add-task-username");
        let password = String::from("can-add-task-password");
        let app =
            test::init_service(build_app!()).await;

        assert_response_success(register(&app, username.as_str(), password.as_str()).await, "Unable to register account");

        let login_response = assert_response_success(login(&app, username.as_str(), password.as_str()).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key, title: String::from("test title"), description: Some(String::from("test description"))
        }).to_request();
        assert_response_success(test::call_service(&app, add_task_request).await, "Unable to add task.");
    }

    #[actix_web::test]
    async fn cannot_add_task_without_valid_auth_key() {
        let app = test::init_service(build_app!()).await;
        let request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: String::from("this-is-not-a-valid-auth-key"), title: String::from("test title"), description: Some(String::from("test description"))
        }).to_request();
        let response = test::call_service(&app, request).await;
        assert_client_error(response, "Response did not indicate client failure");
    }

    #[actix_web::test]
    async fn can_get_task() {
        let username = String::from("can-get-task-username");
        let password = String::from("can-get-task-password");
        let app =
            test::init_service(build_app!()).await;

        assert_response_success(register(&app, username.as_str(), password.as_str()).await, "Unable to register account");

        let login_response = assert_response_success(login(&app, username.as_str(), password.as_str()).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("test title"), description: Some(String::from("test description"))
        }).to_request();
        let add_task_response = assert_response_success(test::call_service(&app, add_task_request).await, "Unable to add task.");

        let task_id = extract_json_i32(add_task_response, "task_id");
        let get_task_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: auth_key, id: task_id
        }).to_request();
        let get_task_response = assert_response_success(test::call_service(&app, get_task_request).await, "Unable to get task.");
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

        assert_response_success(register(&app, owner_username, owner_password).await, "Unable to register owner account");
        assert_response_success(register(&app, non_owner_username, non_owner_password).await, "Unable to register non-owner account");

        let owner_login_response = assert_response_success(login(&app, owner_username, owner_password).await, "Could not login as owner.");
        let owner_auth_key = extract_json_string(owner_login_response, "auth_key");

        let add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_auth_key.clone(), title: String::from("test title"), description: Some(String::from("test description"))
        }).to_request();
        let add_task_response = assert_response_success(test::call_service(&app, add_task_request).await, "Unable to add task.");
        let task_id = extract_json_i32(add_task_response, "task_id");

        let non_owner_login_response = assert_response_success(login(&app, non_owner_username, non_owner_password).await, "Could not login as non-owner.");
        let non_owner_auth_key = extract_json_string(non_owner_login_response, "auth_key");

        let get_task_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: non_owner_auth_key, id: task_id
        }).to_request();
        let get_task_response = test::call_service(&app, get_task_request).await;
        assert!(get_task_response.status().is_client_error(), "Getting task did not indicate client error.");
    }

    #[actix_web::test]
    async fn cannot_get_nonexistent_task() {
        let username = "cannot-get-nonexistent-task-username";
        let password = "cannot-get-nonexistent-task-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account");
        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let get_task_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key, id: -1
        }).to_request();
        let get_task_response = test::call_service(&app, get_task_request).await;
        // A missing task must look the same as a task owned by someone else: 401, not 404.
        assert_eq!(get_task_response.status().as_u16(), 401, "Getting a nonexistent task should not reveal its nonexistence.");
    }

    #[actix_web::test]
    async fn cannot_update_nonexistent_task() {
        let username = "cannot-update-nonexistent-task-username";
        let password = "cannot-update-nonexistent-task-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account");
        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: -1,
            auth_key,
            updates: super::TaskUpdates {
                completed: Some(true),
                title: None,
                description: None,
            },
        }).to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(update_response.status().as_u16(), 401, "Updating a nonexistent task should not reveal its nonexistence.");
    }

    // TODO: Return the updated task so it can be verified
    async fn update_task(username: &str, password: &str, completed: Option<bool>, title: Option<String>, description: Option<String>) -> Task {
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account.");

        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("original title"), description: Some(String::from("original description"))
        }).to_request();
        let add_task_response = assert_response_success(test::call_service(&app, add_task_request).await, "Unable to add task.");
        let task_id = extract_json_i32(add_task_response, "task_id");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: task_id,
            auth_key: auth_key.clone(),
            updates: super::TaskUpdates {
                completed,
                title,
                description,
            },
        }).to_request();
        assert_response_success(test::call_service(&app, update_request).await, "Unable to update task.");

        let get_task_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: auth_key,
            id: task_id,
        }).to_request();
        let get_task_response = assert_response_success(test::call_service(&app, get_task_request).await, "Unable to get task after creating.");
        extract_json_from_constructor(get_task_response, "task", Task::from_json_object).unwrap()
    }

    #[actix_web::test]
    async fn title_is_updatable() {
        let updated_title = String::from("new title");
        let updated_task = update_task("title-is-updateable-username", "title-is-updateable-password", None, Some(updated_title.clone()), None).await;
        assert_eq!(*updated_task.title(), updated_title)
    }

    #[actix_web::test]
    async fn description_is_updatable() {
        let updated_description = String::from("new description");
        let updated_task = update_task("description-is-updateable-username", "description-is-updateable-password", None, None, Some(updated_description.clone())).await;
        assert_eq!(*updated_task.description().as_ref().unwrap(), updated_description)
    }

    #[actix_web::test]
    async fn completed_is_updatable() {
        let updated_completed = true;
        let updated_task = update_task("completed-is-updateable-username", "completed-is-updateable-password", Some(updated_completed), None, None).await;
        assert!(updated_task.completed())
    }

    #[actix_web::test]
    async fn update_modifies_only_the_target_task() {
        let username = "update-only-target-username";
        let password = "update-only-target-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account.");

        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let first_add = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("first task"), description: None
        }).to_request();
        let first_add_response = assert_response_success(test::call_service(&app, first_add).await, "Unable to add first task.");
        let first_task_id = extract_json_i32(first_add_response, "task_id");

        let second_add = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("second task"), description: None
        }).to_request();
        let second_add_response = assert_response_success(test::call_service(&app, second_add).await, "Unable to add second task.");
        let second_task_id = extract_json_i32(second_add_response, "task_id");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: first_task_id,
            auth_key: auth_key.clone(),
            updates: super::TaskUpdates { completed: Some(true), title: None, description: None },
        }).to_request();
        assert_response_success(test::call_service(&app, update_request).await, "Unable to update first task.");

        let get_second = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: auth_key, id: second_task_id
        }).to_request();
        let get_second_response = assert_response_success(test::call_service(&app, get_second).await, "Unable to get second task.");
        let second_task = extract_json_from_constructor(get_second_response, "task", Task::from_json_object).unwrap();
        assert!(!second_task.completed(), "Updating one task changed another task's completion state");
        assert_eq!(*second_task.title(), "second task", "Updating one task changed another task's title");
    }

    #[actix_web::test]
    async fn can_get_all_tasks() {
        let username = "can-get-all-tasks-username";
        let password = "can-get-all-tasks-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account.");

        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let first_add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("task 1"), description: None
        }).to_request();
        assert_response_success(test::call_service(&app, first_add_task_request).await, "Unable to add first task.");

        let second_add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("task 2"), description: None
        }).to_request();
        assert_response_success(test::call_service(&app, second_add_task_request).await, "Unable to add second task.");

        let get_tasks_request = test::TestRequest::get().uri("/all_tasks").set_json(super::GetAllTasksPayload {
            auth_key: auth_key.clone()
        }).to_request();
        let get_tasks_response = assert_response_success(test::call_service(&app, get_tasks_request).await, "Could not get tasks!");
        let tasks: Vec<Task> = extract_json_from_constructor(get_tasks_response, "tasks", |v|
            match v.as_array() {
                Some(a) => a.into_iter().map(|obj| match Task::from_json_object(obj) {
                    Ok(task) => task,
                    Err(e) => panic!("Could not parse task: {e:?}"),
                }).collect(),
                None => panic!("{} is not an array!", v),
            });
        assert_eq!(tasks.len(), 2, "Expected 2 tasks, found {}", tasks.len());
    }

    #[actix_web::test]
    async fn can_get_incomplete_tasks() {
        let username = "can-get-incomplete-tasks-username";
        let password = "can-get-incomplete-tasks-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account.");

        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let first_add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("task 1"), description: None
        }).to_request();
        assert_response_success(test::call_service(&app, first_add_task_request).await, "Unable to add first task.");

        let second_add_task_request = test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: auth_key.clone(), title: String::from("task 2"), description: None
        }).to_request();
        let second_add_task_response = assert_response_success(test::call_service(&app, second_add_task_request).await, "Unable to add second task.");
        let second_task_id = extract_json_i32(second_add_task_response, "task_id");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: second_task_id,
            auth_key: auth_key.clone(),
            updates: super::TaskUpdates {
                completed: Some(true),
                title: None,
                description: None,
            },
        }).to_request();
        assert_response_success(test::call_service(&app, update_request).await, "Unable to mark second task completed.");

        let get_tasks_request = test::TestRequest::get().uri("/incomplete_tasks").set_json(super::GetIncompleteTasksPayload {
            auth_key: auth_key.clone()
        }).to_request();
        let get_tasks_response = assert_response_success(test::call_service(&app, get_tasks_request).await, "Could not get incomplete tasks!");
        let tasks: Vec<Task> = extract_json_from_constructor(get_tasks_response, "tasks", |v|
            match v.as_array() {
                Some(a) => a.into_iter().map(|obj| match Task::from_json_object(obj) {
                    Ok(task) => task,
                    Err(e) => panic!("Could not parse task: {e:?}"),
                }).collect(),
                None => panic!("{} is not an array!", v),
            });
        assert_eq!(tasks.len(), 1, "Expected 1 incomplete task, found {}", tasks.len());
        assert_eq!(*tasks[0].title(), "task 1");
    }

    async fn share_task(app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>, auth_key: &str, id: i32, username: &str, permission: &str) -> ServiceResponse {
        let request = test::TestRequest::post().uri("/share_task").set_json(super::ShareTaskPayload {
            auth_key: String::from(auth_key), id, username: String::from(username), permission: String::from(permission),
        }).to_request();
        test::call_service(app, request).await
    }

    async fn unshare_task(app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>, auth_key: &str, id: i32, username: &str) -> ServiceResponse {
        let request = test::TestRequest::delete().uri("/unshare_task").set_json(super::UnshareTaskPayload {
            auth_key: String::from(auth_key), id, username: String::from(username),
        }).to_request();
        test::call_service(app, request).await
    }

    async fn get_task_shares(app: &impl Service<Request, Response = ServiceResponse, Error = impl std::fmt::Debug>, auth_key: &str, id: i32) -> ServiceResponse {
        let request = test::TestRequest::get().uri("/task_shares").set_json(super::GetTaskSharesPayload {
            auth_key: String::from(auth_key), id,
        }).to_request();
        test::call_service(app, request).await
    }

    #[actix_web::test]
    async fn owner_can_share_and_see_share_listed() {
        let owner_username = "share-list-owner-username";
        let grantee_username = "share-list-grantee-username";
        let password = "share-list-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "Unable to register owner");
        assert_response_success(register(&app, grantee_username, password).await, "Unable to register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read").await, "Share request failed");

        let list_response = assert_response_success(get_task_shares(&app, &owner_key, task_id).await, "list shares");
        let body: serde_json::Value = serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let shares = body["shares"].as_array().expect("shares should be an array");
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

        assert_response_success(register(&app, owner_username, password).await, "Unable to register owner");
        assert_response_success(register(&app, grantee_username, password).await, "Unable to register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read").await, "Share request failed");
        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read_write").await, "Re-share request failed");

        let list_response = assert_response_success(get_task_shares(&app, &owner_key, task_id).await, "list shares");
        let body: serde_json::Value = serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let shares = body["shares"].as_array().expect("shares should be an array");
        assert_eq!(shares.len(), 1, "Re-sharing should replace, not duplicate");
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read_write");
    }

    #[actix_web::test]
    async fn non_owner_cannot_share_unshare_or_list() {
        let owner_username = "nonowner-share-owner-username";
        let non_owner_username = "nonowner-share-nonowner-username";
        let password = "nonowner-share-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, non_owner_username, password).await, "register non-owner");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let non_owner_key = extract_json_string(assert_response_success(login(&app, non_owner_username, password).await, "login non-owner"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_eq!(share_task(&app, &non_owner_key, task_id, non_owner_username, "read").await.status().as_u16(), 401);
        assert_eq!(unshare_task(&app, &non_owner_key, task_id, non_owner_username).await.status().as_u16(), 401);
        assert_eq!(get_task_shares(&app, &non_owner_key, task_id).await.status().as_u16(), 401);
    }

    #[actix_web::test]
    async fn unsharing_removes_the_row() {
        let owner_username = "unshare-owner-username";
        let grantee_username = "unshare-grantee-username";
        let password = "unshare-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, grantee_username, password).await, "register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read").await, "Share request failed");
        assert_response_success(unshare_task(&app, &owner_key, task_id, grantee_username).await, "Unshare request failed");

        let list_response = assert_response_success(get_task_shares(&app, &owner_key, task_id).await, "list shares");
        let body: serde_json::Value = serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let shares = body["shares"].as_array().expect("shares should be an array");
        assert_eq!(shares.len(), 0, "Share row should have been removed");
    }

    #[actix_web::test]
    async fn unknown_target_username_gives_404() {
        let owner_username = "unknown-target-owner-username";
        let password = "unknown-target-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_eq!(share_task(&app, &owner_key, task_id, "no-such-user", "read").await.status().as_u16(), 404);
        assert_eq!(unshare_task(&app, &owner_key, task_id, "no-such-user").await.status().as_u16(), 404);
    }

    #[actix_web::test]
    async fn missing_task_returns_same_401_as_not_owned() {
        let owner_username = "oracle-owner-username";
        let other_username = "oracle-other-username";
        let password = "oracle-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, other_username, password).await, "register other");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let other_key = extract_json_string(assert_response_success(login(&app, other_username, password).await, "login other"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("task"), description: None,
        }).to_request()).await, "add task");
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

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, grantee_username, password).await, "register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let grantee_key = extract_json_string(assert_response_success(login(&app, grantee_username, password).await, "login grantee"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: Some(String::from("original description")),
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read_write").await, "Share request failed");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: task_id,
            auth_key: grantee_key.clone(),
            updates: super::TaskUpdates {
                completed: Some(true),
                title: Some(String::from("updated by grantee")),
                description: Some(String::from("updated description")),
            },
        }).to_request();
        assert_response_success(test::call_service(&app, update_request).await, "read_write grantee should be able to update the task");

        let get_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: owner_key, id: task_id,
        }).to_request();
        let get_response = assert_response_success(test::call_service(&app, get_request).await, "get task");
        let task = extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
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

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, grantee_username, password).await, "register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let grantee_key = extract_json_string(assert_response_success(login(&app, grantee_username, password).await, "login grantee"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read").await, "Share request failed");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: task_id,
            auth_key: grantee_key,
            updates: super::TaskUpdates {
                completed: Some(true),
                title: None,
                description: None,
            },
        }).to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(update_response.status().as_u16(), 401, "read-only sharee must not update the task");

        // The task itself must be unchanged.
        let get_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: owner_key, id: task_id,
        }).to_request();
        let get_response = assert_response_success(test::call_service(&app, get_request).await, "get task");
        let task = extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert!(!task.completed(), "read-only sharee's update must not have been applied");
        assert_eq!(*task.title(), "shared task");
    }

    #[actix_web::test]
    async fn unrelated_user_cannot_update_task() {
        let owner_username = "unrelated-update-owner-username";
        let stranger_username = "unrelated-update-stranger-username";
        let password = "unrelated-update-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, stranger_username, password).await, "register stranger");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let stranger_key = extract_json_string(assert_response_success(login(&app, stranger_username, password).await, "login stranger"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("private task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        let update_request = test::TestRequest::post().uri("/task_by_id").set_json(super::UpdateTaskPayload {
            id: task_id,
            auth_key: stranger_key,
            updates: super::TaskUpdates {
                completed: Some(true),
                title: None,
                description: None,
            },
        }).to_request();
        let update_response = test::call_service(&app, update_request).await;
        assert_eq!(update_response.status().as_u16(), 401, "Unrelated user must not update the task");

        // The task itself must be unchanged.
        let get_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: owner_key, id: task_id,
        }).to_request();
        let get_response = assert_response_success(test::call_service(&app, get_request).await, "get task");
        let task = extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert!(!task.completed(), "Unrelated user's update must not have been applied");
        assert_eq!(*task.title(), "private task");
    }

    #[actix_web::test]
    async fn read_write_sharee_cannot_manage_shares() {
        let owner_username = "rw-manage-owner-username";
        let grantee_username = "rw-manage-grantee-username";
        let password = "rw-manage-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, grantee_username, password).await, "register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let grantee_key = extract_json_string(assert_response_success(login(&app, grantee_username, password).await, "login grantee"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read_write").await, "Share request failed");

        let failed_share = share_task(&app, &grantee_key, task_id, owner_username, "read_write").await;
        assert_eq!(failed_share.status().as_u16(), 401);

        // The failed share call must not have created any share row.
        let list_response = assert_response_success(get_task_shares(&app, &owner_key, task_id).await, "list shares");
        let body: serde_json::Value = serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let shares = body["shares"].as_array().expect("shares should be an array");
        assert_eq!(shares.len(), 1, "Failed share call must not create a new share");
        assert_eq!(shares[0]["username"].as_str().unwrap(), grantee_username);

        let failed_unshare = unshare_task(&app, &grantee_key, task_id, grantee_username).await;
        assert_eq!(failed_unshare.status().as_u16(), 401);

        // The failed unshare call must not have removed the existing share.
        let list_response = assert_response_success(get_task_shares(&app, &owner_key, task_id).await, "list shares");
        let body: serde_json::Value = serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let shares = body["shares"].as_array().expect("shares should be an array");
        assert_eq!(shares.len(), 1, "Failed unshare call must not remove the existing share");
        assert_eq!(shares[0]["username"].as_str().unwrap(), grantee_username);
        assert_eq!(shares[0]["permission"].as_str().unwrap(), "read_write");

        assert_eq!(get_task_shares(&app, &grantee_key, task_id).await.status().as_u16(), 401);
    }

    #[actix_web::test]
    async fn read_share_grants_get_and_listing_access() {
        let owner_username = "read-share-owner-username";
        let grantee_username = "read-share-grantee-username";
        let password = "read-share-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, grantee_username, password).await, "register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let grantee_key = extract_json_string(assert_response_success(login(&app, grantee_username, password).await, "login grantee"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read").await, "Share request failed");

        // The grantee can now fetch the task directly...
        let get_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: grantee_key.clone(), id: task_id,
        }).to_request();
        let get_response = assert_response_success(test::call_service(&app, get_request).await, "grantee should be able to get the shared task");
        let task = extract_json_from_constructor(get_response, "task", Task::from_json_object).unwrap();
        assert_eq!(*task.id(), task_id);

        // ...and it shows up in their listings.
        let list_request = test::TestRequest::get().uri("/all_tasks").set_json(super::GetAllTasksPayload {
            auth_key: grantee_key.clone(),
        }).to_request();
        let list_response = assert_response_success(test::call_service(&app, list_request).await, "list grantee tasks");
        let tasks: Vec<Task> = extract_json_from_constructor(list_response, "tasks", |v|
            match v.as_array() {
                Some(a) => a.iter().map(|obj| Task::from_json_object(obj).unwrap()).collect(),
                None => panic!("not an array"),
            });
        assert_eq!(tasks.len(), 1);
        assert_eq!(*tasks[0].id(), task_id);

        let incomplete_request = test::TestRequest::get().uri("/incomplete_tasks").set_json(super::GetIncompleteTasksPayload {
            auth_key: grantee_key,
        }).to_request();
        let incomplete_response = assert_response_success(test::call_service(&app, incomplete_request).await, "list grantee incomplete tasks");
        let incomplete: Vec<Task> = extract_json_from_constructor(incomplete_response, "tasks", |v|
            match v.as_array() {
                Some(a) => a.iter().map(|obj| Task::from_json_object(obj).unwrap()).collect(),
                None => panic!("not an array"),
            });
        assert_eq!(incomplete.len(), 1);
        assert_eq!(*incomplete[0].id(), task_id);
    }

    #[actix_web::test]
    async fn read_write_share_grants_get_and_listing_access() {
        let owner_username = "rw-share-owner-username";
        let grantee_username = "rw-share-grantee-username";
        let password = "rw-share-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, grantee_username, password).await, "register grantee");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let grantee_key = extract_json_string(assert_response_success(login(&app, grantee_username, password).await, "login grantee"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("shared task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        assert_response_success(share_task(&app, &owner_key, task_id, grantee_username, "read_write").await, "Share request failed");

        let get_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: grantee_key.clone(), id: task_id,
        }).to_request();
        assert_response_success(test::call_service(&app, get_request).await, "read_write grantee should be able to get the task");

        let list_request = test::TestRequest::get().uri("/all_tasks").set_json(super::GetAllTasksPayload {
            auth_key: grantee_key,
        }).to_request();
        let list_response = assert_response_success(test::call_service(&app, list_request).await, "list grantee tasks");
        let tasks: Vec<Task> = extract_json_from_constructor(list_response, "tasks", |v|
            match v.as_array() {
                Some(a) => a.iter().map(|obj| Task::from_json_object(obj).unwrap()).collect(),
                None => panic!("not an array"),
            });
        assert_eq!(tasks.len(), 1);
        assert_eq!(*tasks[0].id(), task_id);
    }

    #[actix_web::test]
    async fn no_share_means_401_and_no_listing_entry() {
        let owner_username = "no-share-owner-username";
        let stranger_username = "no-share-stranger-username";
        let password = "no-share-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");
        assert_response_success(register(&app, stranger_username, password).await, "register stranger");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");
        let stranger_key = extract_json_string(assert_response_success(login(&app, stranger_username, password).await, "login stranger"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key, title: String::from("private task"), description: None,
        }).to_request()).await, "add task");
        let task_id = extract_json_i32(add_response, "task_id");

        let get_request = test::TestRequest::get().uri("/task_by_id").set_json(super::GetTaskByIdPayload {
            auth_key: stranger_key.clone(), id: task_id,
        }).to_request();
        let get_response = test::call_service(&app, get_request).await;
        assert_eq!(get_response.status().as_u16(), 401);

        let list_request = test::TestRequest::get().uri("/all_tasks").set_json(super::GetAllTasksPayload {
            auth_key: stranger_key,
        }).to_request();
        let list_response = assert_response_success(test::call_service(&app, list_request).await, "list stranger tasks");
        let body: serde_json::Value = serde_json::from_slice(&list_response.into_body().try_into_bytes().unwrap()).unwrap();
        let tasks = body["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 0, "Stranger should not see the task in listings");
    }

    #[actix_web::test]
    async fn owner_listings_never_duplicate_with_stale_share_row() {
        let owner_username = "stale-share-owner-username";
        let password = "stale-share-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, owner_username, password).await, "register owner");

        let owner_key = extract_json_string(assert_response_success(login(&app, owner_username, password).await, "login owner"), "auth_key");

        let add_response = assert_response_success(test::call_service(&app, test::TestRequest::post().uri("/add_task").set_json(super::AddTaskPayload {
            auth_key: owner_key.clone(), title: String::from("owned task"), description: None,
        }).to_request()).await, "add task");
        let owned_task_id = extract_json_i32(add_response, "task_id");

        // Simulate a stale share row granting the owner access to their own
        // task by inserting it directly; the owner's listing must not
        // duplicate the task.
        let mut connection = super::establish_connection().expect("db connection");
        let stale_owner = super::auth_key_to_user(&owner_key, &mut connection).unwrap();
        use crate::schema::task_shares::dsl::*;
        use diesel::prelude::*;
        diesel::insert_into(task_shares)
            .values((task_id.eq(&owned_task_id), user_id.eq(stale_owner.ref_id()), permission.eq(crate::core::SharePermission::Read)))
            .execute(&mut connection)
            .expect("insert stale share row");
        drop(connection);

        let list_request = test::TestRequest::get().uri("/all_tasks").set_json(super::GetAllTasksPayload {
            auth_key: owner_key.clone(),
        }).to_request();
        let list_response = assert_response_success(test::call_service(&app, list_request).await, "list owner tasks");
        let tasks: Vec<Task> = extract_json_from_constructor(list_response, "tasks", |v|
            match v.as_array() {
                Some(a) => a.iter().map(|obj| Task::from_json_object(obj).unwrap()).collect(),
                None => panic!("not an array"),
            });
        assert_eq!(tasks.len(), 1, "Owner's task should appear exactly once even with a stale share row");
    }

    #[actix_web::test]
    async fn user_by_id_returns_username_for_existing_user() {
        let username = "user-by-id-existing-username";
        let password = "user-by-id-existing-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account");
        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        // Look up the user's id directly (usernames are the public handle;
        // task/share payloads carry the id).
        let user_id = {
            let mut connection = super::establish_connection().expect("db connection");
            let un = crate::domain_types::Username::new(String::from(username)).unwrap();
            let user = crate::core::get_user_by_name(&un, &mut connection).unwrap();
            *user.ref_id()
        };

        let request = test::TestRequest::get().uri("/user_by_id").set_json(super::GetUserByIdPayload {
            auth_key,
            id: user_id,
        }).to_request();
        let response = assert_response_success(test::call_service(&app, request).await, "User by id lookup failed.");
        let response_name = extract_json_string(response, "username");
        assert_eq!(username, response_name);
    }

    #[actix_web::test]
    async fn user_by_id_returns_404_for_unknown_id() {
        let username = "user-by-id-unknown-username";
        let password = "user-by-id-unknown-password";
        let app = test::init_service(build_app!()).await;

        assert_response_success(register(&app, username, password).await, "Unable to register account");
        let login_response = assert_response_success(login(&app, username, password).await, "Could not login.");
        let auth_key = extract_json_string(login_response, "auth_key");

        let request = test::TestRequest::get().uri("/user_by_id").set_json(super::GetUserByIdPayload {
            auth_key,
            id: -1,
        }).to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status().as_u16(), 404, "Unknown user id should return 404.");
    }
}
