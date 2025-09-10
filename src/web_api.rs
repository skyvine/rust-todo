use crate::core::{
    add_user,
    auth_key_to_user,
    establish_connection,
    get_new_auth_key,
    get_user_by_name,
    ApplicationError,
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
                HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": request_id})))
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
    HttpResponse::NotImplemented().finish()
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct GetTaskByIdPayload {
    auth_key: String,
    id: i32,
}

/// Returns a specific task based on the ID of the given task.
/// 
/// The user must be logged in as the task's owner in order to get the task.
#[get("/task_by_id")]
pub async fn get_task_by_id(payload: actix_web::web::Json<GetTaskByIdPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Get Task by ID", %request_id).entered();


    match establish_connection() {
        Ok(mut connection) => {
            match crate::core::get_task_by_id(&payload.id, &mut connection) {
                Ok(task) => {
                    let user = match auth_key_to_user(&payload.auth_key, &mut connection) {
                        Ok(user) => user,
                        Err(e) => return e.into_http_response(&format!("{request_id}")),
                    };

                    if user.ref_id() == task.owner_id() {
                        HttpResponse::Ok().body(format!("{}", json!({"request_id": format!("{request_id}"), "task": task})))
                    } else {
                        event!(Level::ERROR, "Cannot get task {}, belongs to user {} but requested by user {}", task.id(), task.owner_id(), user.ref_id());
                        HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                    }
                }
                Err(e) => e.into_http_response(&format!("{request_id}")),
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
    let description = TaskDescription::new(std::mem::take(payload.description.as_mut().unwrap_or(&mut String::default())));

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}"))
            };

            match crate::core::add_task(&owner, &title, false, &description, &mut connection) {
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
                Ok(auth_key) => HttpResponse::Ok().body(format!("{}", json!({ "auth_key": format!("{auth_key}")}))),
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

#[derive(Deserialize, Serialize)]
struct UpdateTaskPayload {
    id: i32,
    auth_key: String,
    completed: Option<bool>,
    title: Option<String>,
    description: Option<String>,
}

/// Changes one or more of the task's fields.
/// 
/// It is an error for all of the optional values to be None; at least on field must be updated for
/// this endpoint to succeed.
#[post("/task_by_id")]
pub async fn update_task_by_id(mut payload: actix_web::web::Json<UpdateTaskPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Update Task", %request_id).entered();

    let title = if let Some(title) = payload.title.as_mut() {
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

    let description = payload.description.as_mut().map(|d| TaskDescription::new(std::mem::take(d)));

    match establish_connection() {
        Ok(mut connection) => {
            let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
                Ok(user) => user,
                Err(e) => return e.into_http_response(&format!("{request_id}")),
            };

            match crate::core::update_task(&owner, &payload.id, payload.completed, title, description, &mut connection) {
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
        if !response.status().is_client_error() {
            let formatted_response = format!("{response:?}");
            let body = response.into_body();
            panic!("Response did not indicate client failure: {formatted_response}{body:?}")
        }
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
        if !response.status().is_client_error() {
            let formatted_response = format!("{response:?}");
            let body = response.into_body();
            panic!("Response did not indicate client error: {formatted_response}{body:?}")
        }
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
        if !response.status().is_client_error() {
            let formatted_response = format!("{response:?}");
            let body = response.into_body();
            panic!("Response did not indicate client failure: {formatted_response}{body:?}")
        }
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
            completed: completed,
            title: title,
            description: description,
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
}
