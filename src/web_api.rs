use crate::database::{
    self,

    auth_key_to_user,
    establish_connection,
    user_exists,
};
use crate::domain_types::{CleartextPassword, TaskDescription, TaskTitle, Username};

use actix_web::{get, post, HttpResponse, Responder};
use argon2::{
    password_hash::{
        rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString
    },
    Argon2
};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{event, span, Level};
use uuid::Uuid;
use zeroize::{ZeroizeOnDrop};

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
    use crate::schema::users::dsl::*;

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

            match user_exists(un.as_ref(), &mut conn) {
                Ok(false) => {
                    let query =
                        diesel::insert_into(users).values((username.eq(un.as_ref()), password.eq(hashed_password)));

                    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

                    let result = query.execute(&mut conn);
                    match result {
                        Ok(_) => {
                            event!(Level::TRACE, "Query succeeded");
                            HttpResponse::Created().finish()
                        },
                        Err(e) => {
                            event!(Level::ERROR, "Query failed: {e}");
                            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                        }
                    }
                },

                Ok(true) => HttpResponse::Conflict().body(format!("{}", json!({"request_id": format!("{request_id}")}))),

                Err(e) => {
                    event!(Level::ERROR, "Unable to query database for existing users: {e}");
                    HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                }
            }
        },

        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}.");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct WhoAmIPayload {
    auth_key: String
}

#[get("/whoami")]
pub async fn whoami(payload: actix_web::web::Json<WhoAmIPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Who Am I Request", %request_id).entered();

    match establish_connection() {
        Ok(mut conn) => 
            match auth_key_to_user(&payload.auth_key, &mut conn) {
                Ok(user) => HttpResponse::Ok().body(format!("{}", json!({ "request_id": format!("{}", request_id), "username": user.ref_username()}))),
                Err(e) => {
                    event!(Level::ERROR, "Unable to look up user: {e}");
                    HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{}", request_id)})))
                }
            },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{}", request_id)})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct LoginPayload {
    username: String,
    password: String
}

#[post("/login")]
pub async fn login(mut payload: actix_web::web::Json<LoginPayload>) -> impl Responder {
    use crate::schema::auth_keys::dsl::*;
    use crate::schema::users::dsl::*;

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
            let query = users
                .filter(username.eq(un.as_ref()))
                .select(database::User::as_select());
            event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

            match query.load::<database::User>(&mut conn) {
                Ok(found_users) => {
                    let user = &found_users[0];

                    let hashed_password = match PasswordHash::new(user.ref_password()) {
                        Ok(ph) => ph,
                        Err(e) => {
                            event!(Level::ERROR, "Unable to parse hashed password ({}): {e:?}", user.ref_password());
                            return HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})));
                        }
                    };

                    match Argon2::default().verify_password(pw.as_ref().as_bytes(), &hashed_password) {
                        Ok(_) => {
                            let new_key = Uuid::new_v4();
                            let query = diesel::insert_into(auth_keys)
                                .values((user_id.eq(user.ref_id()), key.eq(format!("{new_key}"))));
                            event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

                            match query.execute(&mut conn) {
                                Ok(_) => {
                                    HttpResponse::Ok().body(format!("{}", json!({ "auth_key": format!("{}", new_key)})))
                                },
                                Err(e) => {
                                    event!(Level::ERROR, "Unable to insert new auth key: {e}");
                                    HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                                }

                            }
                        },
                        Err(_) => {
                            HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                        }
                    }
                },
                Err(e) => {
                    event!(Level::ERROR, "Query failed: {e}");
                    HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                }
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{}", request_id)})))
        }
    }
}

#[derive(Deserialize, Serialize, ZeroizeOnDrop)]
struct AddTaskPayload {
    auth_key:    String,
    title:       String,
    description: Option<String>,
}

#[post("/add_task")]
pub async fn add_task(mut payload: actix_web::web::Json<AddTaskPayload>) -> impl Responder {
    use crate::schema::tasks::dsl;

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

    let mut connection = match establish_connection() {
        Ok(connection) => connection,
        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e:?}");
            return HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})));
        }
    };

    let owner = match auth_key_to_user(&payload.auth_key, &mut connection) {
        Ok(user) => user,
        Err(e) => {
            event!(Level::ERROR, "Unable to retrieve user based on auth key: {e:?}");
            return HttpResponse::Unauthorized().body(format!("{}", json!({"request_id": format!("{request_id}")})));
        }
    };

    let query =
        diesel::insert_into(dsl::tasks).values((dsl::owner.eq(owner.ref_id()), dsl::title.eq(title.as_ref()), dsl::description.eq(description.as_ref())));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.execute(&mut connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            HttpResponse::Created().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_http::Request;
    use actix_web::{body::MessageBody, dev::Service, dev::ServiceResponse, test, App};
    use crate::build_app;
    use ctor::ctor;
    use tracing_subscriber::{fmt, EnvFilter};

    #[ctor]
    unsafe fn global_init() {
        use crate::schema::auth_keys::dsl::*;
        use crate::schema::tasks::dsl::*;
        use crate::schema::users::dsl::*;
        use super::RunQueryDsl;

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

    fn extract_json_string(response: ServiceResponse, key: &str) -> String {
        match response.into_body().try_into_bytes() {
            Ok(bytes) =>
                match serde_json::from_slice::<serde_json::Value>(bytes.as_ref()) {
                    Ok(dict) => {
                        match dict[key].as_str() {
                            Some(s) => String::from(s),
                            None => panic!("{} is not a string! {:?}", key, dict[key]),
                        }
                    },
                    Err(_) => panic!("Unable to deserialize alleged JSON: {bytes:?}"),
                },
            Err(e) => {
                panic!("Unable to extract bytes from response {e:?}")
            }
        }
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
}
