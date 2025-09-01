use crate::domain_types::{CleartextPassword, TaskDescription, TaskTitle, Username};
use argon2::{
    password_hash::PasswordVerifier,
    Argon2
};
use diesel::prelude::*;
use std::env;
use tracing::{event, Level};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub enum ApplicationDatabaseError {
    DieselError(diesel::result::Error),
    InvalidPassword,
    QueryFailed(String),
    UserExists,
}

/// A complete entry from the auth_keys table in the database
#[derive(Queryable, Selectable, ZeroizeOnDrop)]
#[diesel(table_name = crate::schema::auth_keys)]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AuthKey {
    id:      i32,
    user_id: i32,
    key:     String
}

/// A complete entry from the tasks table in the database
#[allow(dead_code)]
#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::tasks)]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Task {
    id:          i32,
    owner:       i32,
    title:       String,
    description: Option<String>,
}

/// A complete entry from the users table in the database
#[derive(Clone, Queryable, Selectable, ZeroizeOnDrop)]
#[diesel(table_name = crate::schema::users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    id: i32,
    username: String,
    password: String,
}

impl User {
    pub fn ref_id(&self) -> &i32 {
        &self.id
    }

    pub fn ref_username(&self) -> &String {
        &self.username
    }

    pub fn ref_password(&self) -> &String {
        &self.password
    }
}

pub fn add_task(owner: &User, title: &TaskTitle, description: &TaskDescription, connection: &mut PgConnection) -> Result<(), ApplicationDatabaseError> {
    use crate::schema::tasks::dsl;

    let query =
        diesel::insert_into(dsl::tasks).values((dsl::owner.eq(owner.ref_id()), dsl::title.eq(title.as_ref()), dsl::description.eq(description.as_ref())));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.execute(connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationDatabaseError::DieselError(e))
        }
    }
}

pub fn add_user(un: &Username, hashed_password: &String, connection: &mut PgConnection) -> Result<(), ApplicationDatabaseError> {
    use crate::schema::users::dsl::*;

    match user_exists(un.as_ref(), connection) {
        Ok(false) => {
            let query =
                diesel::insert_into(users).values((username.eq(un.as_ref()), password.eq(hashed_password)));

            event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

            let result = query.execute(connection);
            match result {
                Ok(_) => {
                    event!(Level::TRACE, "Query succeeded");
                    Ok(())
                },
                Err(e) => {
                    Err(ApplicationDatabaseError::QueryFailed(format!("{e}")))
                }
            }
        },

        Ok(true) => Err(ApplicationDatabaseError::UserExists),

        Err(e) => Err(e)
    }
}

pub fn auth_key_to_user(auth_key: &String, connection: &mut PgConnection) -> Result<User, diesel::result::Error> {
    use crate::schema::auth_keys::dsl::*;
    use crate::schema::users::dsl::*;

    let query = auth_keys.inner_join(users)
        .filter(key.eq(auth_key))
        .select(( AuthKey::as_select(), User::as_select()));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    let results = query.load::<(AuthKey, User)>(connection)?;
    if results.is_empty() {
        Err(diesel::result::Error::NotFound)
    } else {
        Ok(results[0].1.clone())
    }
}

/// Open a new connection to the database. The DATABASE_URL environment variable must be defined and
/// point to a running database.
pub fn establish_connection() -> Result<PgConnection, ConnectionError> {
    let mut database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            let msg = String::from("DATABASE_URL is not set, unable to establish connection");
            event!(Level::ERROR, msg);
            return Err(ConnectionError::InvalidConnectionUrl(msg))
        }
    };

    let connection = PgConnection::establish(&database_url);
    database_url.zeroize();
    connection
}

pub fn get_new_auth_key(user: &User, given_password: &CleartextPassword, hashed_password: &argon2::PasswordHash, connection: &mut PgConnection) -> Result<Uuid, ApplicationDatabaseError> {
    use crate::schema::auth_keys::dsl::*;

    match Argon2::default().verify_password(given_password.as_ref().as_bytes(), hashed_password) {
        Ok(_) => {
            let new_key = Uuid::new_v4();
            let query = diesel::insert_into(auth_keys)
                .values((user_id.eq(user.ref_id()), key.eq(format!("{new_key}"))));
            event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

            match query.execute(connection) {
                Ok(_) => Ok(new_key),
                Err(e) => {
                    event!(Level::ERROR, "Unable to insert new auth key: {e}");
                    Err(ApplicationDatabaseError::DieselError(e))
                }

            }
        },
        Err(_) => {
            Err(ApplicationDatabaseError::InvalidPassword)
        }
    }

}

pub fn get_user_by_name(un: &Username, connection: &mut PgConnection) -> Result<User, ApplicationDatabaseError> {
    use crate::schema::users::dsl::*;

    let query = users
        .filter(username.eq(un.as_ref()))
        .select(User::as_select());
    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<User>(connection) {
        Ok(found_users) => {
            Ok(found_users[0].clone())
        },
        Err(e) => {
            event!(Level::ERROR, "Query failed while getting user by name: {e}");
            Err(ApplicationDatabaseError::DieselError(e))
        }
    }
}

/// Returns true if a user with the given name exists, false otherwise.
pub fn user_exists(name: &String, connection: &mut PgConnection) -> Result<bool, ApplicationDatabaseError> {
    use crate::schema::users::dsl::*;
    let query = users.filter(username.eq(name)).select(User::as_select());
    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    match query.load(connection) {
        Ok(collection) => Ok(!collection.is_empty()),
        Err(e) => Err(ApplicationDatabaseError::DieselError(e))
    }
}
