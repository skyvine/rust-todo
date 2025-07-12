mod schema;

use actix_web::{get, post, App, HttpResponse, HttpServer, Responder};
use argon2::{
    password_hash::{
        rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString
    },
    Argon2
};
use diesel::prelude::*;
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use tracing::{event, span, Level};
use uuid::Uuid;

/// An endpoint to checks that the server is up.
/// 
/// This endpoint always responds with status OK simply to verify that
/// the server is running and responding to requests.
#[get("/is_alive")]
async fn is_alive() -> impl Responder {
    event!(Level::TRACE, "Responding to is_alive check");
    HttpResponse::Ok()
}

// This struct is needed to set the return type in the query in user_exists, but the fields are not
// (yet) used. Ignore the dead code warning because the fields need to exist for diesel to validate
// the struct.
#[allow(dead_code)]
#[derive(Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct User {
    id: i32,
    username: String,
    password: String,
}

// Again, this struct exists so that queries can be made but not all of the members are currently
// used.
#[allow(dead_code)]
#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::auth_keys)]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct AuthKey {
    id:      i32,
    user_id: i32,
    key:     String
}

/// Open a new connection to the database. The DATABASE_URL environment variable must be defined and
/// point to a running database.
fn establish_connection() -> Result<PgConnection, ConnectionError> {
    let database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            let msg = String::from("DATABASE_URL is not set, unable to establish connection");
            event!(Level::ERROR, msg);
            return Err(ConnectionError::InvalidConnectionUrl(msg))
        }
    };

    PgConnection::establish(&database_url)
}

/// Returns true if a user with the given name exists, false otherwise.
fn user_exists(name: &String, connection: &mut PgConnection) -> Result<bool, diesel::result::Error> {
    use self::schema::users::dsl::*;
    let query = users.filter(username.eq(name)).select(User::as_select());
    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    Ok(!query.load(connection)?.is_empty())
}

#[derive(Deserialize, Serialize)]
struct UserRegistrationPayload {
    name: String,
    password: String,
}

/// Create an account with the given account info.
/// 
/// On a success, the user will be added to the database as a valid user and return HTTP OK.
/// 
/// In any error state this will create a log event at the ERROR level and return an appropriate
/// error HTTP response to send back to the client.
#[post("/register_account")]
async fn register_account(mut account_info: actix_web::web::Json<UserRegistrationPayload>) -> impl Responder {
    use self::schema::users::dsl::*;

    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Registering Account", %request_id).entered();

    let connection = establish_connection();

    match connection {
        Ok(mut conn) => {
            event!(Level::TRACE, "Established connection to database.");

            // move not allowed in .values() call below, avoid cloning by replacing
            let un = std::mem::take(&mut account_info.name);
            let pw = std::mem::take(&mut account_info.password);

            match user_exists(&un, &mut conn) {
                Ok(false) => {
                    let slt = SaltString::generate(&mut OsRng);

                    let hashed_password = match Argon2::default().hash_password(pw.as_bytes(), &slt) {
                        Ok(h) => h.to_string(),
                        Err(e) => {
                            event!(Level::ERROR, "Unable to hash password: {e:?}");
                            return HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})));
                        }
                    };

                    let query =
                        diesel::insert_into(users).values((username.eq(un), password.eq(hashed_password)));

                    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

                    let result = query.execute(&mut conn);
                    match result {
                        Ok(_) => {
                            event!(Level::TRACE, "Query succeeded");
                            HttpResponse::Ok().finish()
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

fn auth_key_to_user(auth_key: &String, connection: &mut PgConnection) -> Result<User, diesel::result::Error> {
    use crate::schema::auth_keys::dsl::*;
    use crate::schema::users::dsl::*;

    let query = auth_keys.inner_join(users)
        .filter(key.eq(auth_key))
        .select(( AuthKey::as_select(), User::as_select()));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    Ok(query.load::<(AuthKey, User)>(connection)?[0].1.clone())
}

#[derive(Deserialize, Serialize)]
struct WhoAmIPayload {
    auth_key: String
}

#[get("/whoami")]
async fn whoami(payload: actix_web::web::Json<WhoAmIPayload>) -> impl Responder {
    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Who Am I Request", %request_id).entered();

    let connection = establish_connection();

    match connection {
        Ok(mut conn) => 
            match auth_key_to_user(&payload.auth_key, &mut conn) {
                Ok(user) => HttpResponse::Ok().body(format!("{}", json!({ "request_id": format!("{}", request_id), "username": user.username}))),
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

#[derive(Deserialize, Serialize)]
struct LoginPayload {
    username: String,
    password: String
}

#[post("/login")]
async fn login(payload: actix_web::web::Json<LoginPayload>) -> impl Responder {
    use crate::schema::auth_keys::dsl::*;
    use crate::schema::users::dsl::*;

    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Login", %request_id).entered();

    let connection = establish_connection();

    match connection {
        Ok(mut conn) => {
            let query = users
                .filter(username.eq(&payload.username))
                .select(User::as_select());
            event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

            match query.load::<User>(&mut conn) {
                Ok(found_users) => {
                    let user = &found_users[0];

                    let hashed_password = match PasswordHash::new(&user.password) {
                        Ok(ph) => ph,
                        Err(e) => {
                            event!(Level::ERROR, "Unable to parse hashed password ({}): {e:?}", user.password);
                            return HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})));
                        }
                    };

                    match Argon2::default().verify_password(payload.password.as_bytes(), &hashed_password) {
                        Ok(_) => {
                            let new_key = Uuid::new_v4();
                            let query = diesel::insert_into(auth_keys)
                                .values((user_id.eq(user.id), key.eq(format!("{new_key}"))));
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

/// Run the server on the given IP address and port.
pub async fn run(ip_address: String, port: u16) -> std::io::Result<()> {
    dotenv().ok();
    HttpServer::new(|| {
            App::new()
            .service(is_alive)
        })
    .bind((ip_address, port))?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use actix_http::Request;
    use actix_web::{body::MessageBody, dev::Service, dev::ServiceResponse, test, App};
    use ctor::ctor;
    use tracing::Level;
    use tracing_subscriber::fmt;

    #[ctor]
    unsafe fn global_init() {
        use super::schema::auth_keys::dsl::*;
        use super::schema::users::dsl::*;
        use super::RunQueryDsl;

        // Print log messages to help debug failed tests
        fmt().event_format(fmt::format().pretty()).with_max_level(Level::TRACE).init();

        // The tests depend on the state of the database, so ensure we always start with a clean slate.
        diesel::delete(auth_keys).execute(&mut super::establish_connection().expect("Unable to connect to test database."));
        diesel::delete(users).execute(&mut super::establish_connection().expect("Unable to connect to test database."));
    }

    /// Call the service, but panic if the response does not indicate success
    async fn try_call_service<App: Service<Request, Response = ServiceResponse>>(app: &App, request: Request, message: &str) -> ServiceResponse
    where <App as Service<Request>>::Error: std::fmt::Debug
    {
        let response = test::call_service(app, request).await;
        if !response.status().is_success() {
            let formatted_response = format!("{response:?}");
            let body = response.into_body();
            panic!("{message}: {formatted_response}{body:?}")
        } else {
            response
        }
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
        let app = test::init_service(App::new().service(super::is_alive)).await;
        let request = test::TestRequest::get().uri("/is_alive").to_request();
        let response = try_call_service(&app, request, "Is alive check failed").await;
        assert_eq!(response.into_body().size(), actix_web::body::BodySize::Sized(0));
    }

    #[actix_web::test]
    async fn regitering_account_is_successful() {
        let app = test::init_service(App::new().service(super::register_account)).await;
        let request = test::TestRequest::post().uri("/register_account").set_json(super::UserRegistrationPayload {
            name:     String::from("new-name"),
            password: String::from("new-password")
        }).to_request();
        try_call_service(&app, request, "Registration request failed").await;
    }

    #[actix_web::test]
    async fn duplicate_username_fails() {
        let name = String::from("duplicated-name");
        let password = String::from("duplicated-password");

        let app = test::init_service(App::new().service(super::register_account)).await;
        let first_request = test::TestRequest::post().uri("/register_account").set_json(super::UserRegistrationPayload {
            name:     name.clone(),
            password: password.clone(),
        }).to_request();
        try_call_service(&app, first_request, "Registration failed").await;

        let second_request = test::TestRequest::post().uri("/register_account").set_json(super::UserRegistrationPayload {
            name,
            password,
        }).to_request();
        let second_response = test::call_service(&app, second_request).await;
        if !second_response.status().is_client_error() {
            let formatted_response = format!("{second_response:?}");
            let body = second_response.into_body();
            panic!("Response indicated success: {formatted_response}{body:?}")
        }
    }

    #[actix_web::test]
    async fn auth_key_is_recognized() {
        let name = String::from("auth-key-registered-name");
        let password = String::from("auth-key-registered-password");
        let app =
            test::init_service(App::new()
                .service(super::login)
                .service(super::register_account)
                .service(super::whoami))
                .await;

        let register_request = test::TestRequest::post().uri("/register_account").set_json(super::UserRegistrationPayload {
            name: name.clone(),
            password: password.clone(),
        }).to_request();
        try_call_service(&app, register_request, "Unable to register account").await;

        let login_request = test::TestRequest::post().uri("/login").set_json(super::LoginPayload {
            username: name.clone(),
            password: password.clone(),
        }).to_request();
        let login_response = try_call_service(&app, login_request, "Unable to login").await;
        let auth_key = extract_json_string(login_response, "auth_key");

        let whoami_request = test::TestRequest::get().uri("/whoami").set_json(super::WhoAmIPayload {
            auth_key,
        }).to_request();
        let whoami_response = try_call_service(&app, whoami_request, "Whoami request failed").await;

        let response_name = extract_json_string(whoami_response, "username");
        assert_eq!(name, response_name);
    }
}
