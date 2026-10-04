use crate::domain_types::{CleartextPassword, TaskDescription, TaskTitle, Username};
use argon2::{
    password_hash::PasswordVerifier,
    Argon2
};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::env;
use tracing::{event, Level};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug)]
pub enum ApplicationError {
    DieselError(diesel::result::Error),
    InvalidAuthKey,
    InvalidData(String),
    InvalidPassword,
    QueryFailed(String),
    Unauthorized,
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
#[derive(Clone, Debug, Deserialize, Queryable, Selectable, Serialize)]
#[diesel(table_name = crate::schema::tasks)]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Task {
    id:          i32,
    owner:       i32,
    title:       String,
    description: Option<String>,
    completed:   bool,
}

#[allow(dead_code)]
impl Task {
    // Constructors
    pub fn from_json_object(object: &serde_json::Value) -> Result<Self, ApplicationError> {
        let id = match object["id"].as_i64() {
            Some(number) => number,
            None => return Err(ApplicationError::InvalidData(format!("{} should be a number", object["id"]))),
        } as i32;

        let owner = match object["owner"].as_i64() {
            Some(number) => number,
            None => return Err(ApplicationError::InvalidData(format!("{} should be a number", object["owner"]))),
        } as i32;

        let completed = match object["completed"].as_bool() {
            Some(b) => b,
            None => return Err(ApplicationError::InvalidData(format!("{} should be a bool", object["completed"]))),
        };

        let title = String::from(match object["title"].as_str() {
            Some(s) => s,
            None => return Err(ApplicationError::InvalidData(format!("{} should be a string", object["title"])))
        });

        let description = match object["description"].as_str() {
            Some(s) => Some(String::from(s)),
            None => {
                if object["description"].is_null() {
                    None
                } else {
                    return Err(ApplicationError::InvalidData(format!("{} should be a string", object["description"])))
                }
            }
        };

        Ok(Task {
            id,
            owner,
            completed,
            title,
            description,
        })
    }

    // Accessors
    pub fn id(&self) -> &i32 {
        &self.id
    }

    pub fn owner_id(&self) -> &i32 {
        &self.owner
    }

    pub fn completed(&self) -> &bool {
        &self.completed
    }

    pub fn title(&self) -> &String {
        &self.title
    }

    pub fn description(&self) -> &Option<String> {
        &self.description
    }
}

#[allow(dead_code)]
#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::tasks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TaskUpdate {
    completed:   Option<bool>,
    title:       Option<String>,
    description: Option<String>,
}

/// The level of access granted by a task share, stored as text in `task_shares.permission`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, diesel::expression::AsExpression, diesel::deserialize::FromSqlRow)]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum SharePermission {
    Read,
    ReadWrite,
}

impl SharePermission {
    /// Parse the stored text value (`"read"` or `"read_write"`).
    pub fn from_stored_text(text: &str) -> Result<Self, ApplicationError> {
        match text {
            "read" => Ok(SharePermission::Read),
            "read_write" => Ok(SharePermission::ReadWrite),
            other => Err(ApplicationError::InvalidData(format!("Unknown share permission: {other}"))),
        }
    }

    /// The text value stored in the database.
    pub fn as_stored_text(&self) -> &'static str {
        match self {
            SharePermission::Read => "read",
            SharePermission::ReadWrite => "read_write",
        }
    }
}

impl diesel::deserialize::FromSql<diesel::sql_types::Text, diesel::pg::Pg> for SharePermission {
    fn from_sql(bytes: diesel::pg::PgValue<'_>) -> diesel::deserialize::Result<Self> {
        let text = <String as diesel::deserialize::FromSql<diesel::sql_types::Text, diesel::pg::Pg>>::from_sql(bytes)?;
        SharePermission::from_stored_text(&text).map_err(|e| match e {
            ApplicationError::InvalidData(msg) => msg.into(),
            other => format!("{other:?}").into(),
        })
    }
}

impl diesel::serialize::ToSql<diesel::sql_types::Text, diesel::pg::Pg> for SharePermission {
    fn to_sql<'b>(&'b self, out: &mut diesel::serialize::Output<'b, '_, diesel::pg::Pg>) -> diesel::serialize::Result {
        use std::io::Write;
        out.write_all(self.as_stored_text().as_bytes())?;
        Ok(diesel::serialize::IsNull::No)
    }
}

/// A complete entry from the task_shares table in the database
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Queryable, Selectable, Serialize)]
#[diesel(table_name = crate::schema::task_shares)]
#[diesel(belongs_to(Task))]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TaskShare {
    id:         i32,
    task_id:    i32,
    user_id:    i32,
    permission: SharePermission,
}

#[allow(dead_code)]
impl TaskShare {
    pub fn id(&self) -> &i32 {
        &self.id
    }

    pub fn task_id(&self) -> &i32 {
        &self.task_id
    }

    pub fn user_id(&self) -> &i32 {
        &self.user_id
    }

    pub fn permission(&self) -> SharePermission {
        self.permission
    }
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

/// What a user is allowed to do with a particular task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Owner,
    ReadWrite,
    ReadOnly,
    None,
}

/// Resolve the effective permission a user has on a task.
///
/// - `Permission::Owner` if the user owns the task,
/// - otherwise the share row for (task_id, user_id), mapped
///   `read` -> `ReadOnly`, `read_write` -> `ReadWrite`,
/// - `Permission::None` when there is no share row.
///
/// Returns `ApplicationError::DieselError(NotFound)` if the task does not
/// exist, so callers can keep hiding task existence behind a uniform 401.
pub fn get_task_permission(user: &User, task_id: &i32, connection: &mut PgConnection) -> Result<Permission, ApplicationError> {
    let task = get_task_by_id(task_id, connection)?;

    if task.owner_id() == user.ref_id() {
        return Ok(Permission::Owner);
    }

    use crate::schema::task_shares::dsl::{self, task_shares};

    let query = task_shares
        .filter(dsl::task_id.eq(task_id))
        .filter(dsl::user_id.eq(user.ref_id()))
        .select(TaskShare::as_select());

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<TaskShare>(connection) {
        Ok(rows) => {
            match rows.into_iter().next() {
                Some(share) => match share.permission() {
                    SharePermission::Read => Ok(Permission::ReadOnly),
                    SharePermission::ReadWrite => Ok(Permission::ReadWrite),
                },
                None => Ok(Permission::None),
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Query failed while looking up task share: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn add_task(owner: &User, title: &TaskTitle, completed: bool, description: Option<&TaskDescription>, connection: &mut PgConnection) -> Result<Task, ApplicationError> {
    use crate::schema::tasks::dsl;

    let query =
        diesel::insert_into(dsl::tasks).values((dsl::owner.eq(owner.ref_id()),
                                                                dsl::title.eq(title.as_ref()),
                                                                dsl::description.eq(description.map(AsRef::as_ref)),
                                                                dsl::completed.eq(completed),
                                                            ));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.get_result(connection) {
        Ok(task) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(task)
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn add_user(un: &Username, hashed_password: &String, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    use crate::schema::users::dsl::*;

    let query =
        diesel::insert_into(users).values((username.eq(un.as_ref()), password.eq(hashed_password)));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.execute(connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        },
        Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _)) => {
            event!(Level::ERROR, "Unable to add user: username already exists");
            Err(ApplicationError::UserExists)
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn auth_key_to_user(auth_key: &String, connection: &mut PgConnection) -> Result<User, ApplicationError> {
    use crate::schema::auth_keys::dsl::*;
    use crate::schema::users::dsl::*;

    let query = auth_keys.inner_join(users)
        .filter(key.eq(auth_key))
        .filter(expiration.gt(diesel::dsl::now))
        .select(( AuthKey::as_select(), User::as_select()));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<(AuthKey, User)>(connection) {
        Ok(results) => {
            if results.is_empty() {
                Err(ApplicationError::InvalidAuthKey)
            } else {
                Ok(results[0].1.clone())
            }
        },
        Err(e) => Err(ApplicationError::DieselError(e))
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

pub fn get_all_tasks_for_user(user: &User, connection: &mut PgConnection) -> Result<Vec<Task>, ApplicationError> {
    use crate::schema::tasks::dsl as task_dsl;
    use crate::schema::task_shares::dsl as share_dsl;

    // Tasks shared with the user, by id, as a subquery.
    let shared_task_ids = share_dsl::task_shares
        .filter(share_dsl::user_id.eq(user.ref_id()))
        .select(share_dsl::task_id);

    let query = task_dsl::tasks
        .filter(task_dsl::owner.eq(user.ref_id()).or(task_dsl::id.eq_any(shared_task_ids)))
        .select(Task::as_select());

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<Task>(connection) {
        Ok(found_tasks) => Ok(found_tasks),
        Err(e) => Err(ApplicationError::DieselError(e)),
    }
}

pub fn get_incomplete_tasks_for_user(user: &User, connection: &mut PgConnection) -> Result<Vec<Task>, ApplicationError> {
    use crate::schema::tasks::dsl as task_dsl;
    use crate::schema::task_shares::dsl as share_dsl;

    // Tasks shared with the user, by id, as a subquery.
    let shared_task_ids = share_dsl::task_shares
        .filter(share_dsl::user_id.eq(user.ref_id()))
        .select(share_dsl::task_id);

    let query = task_dsl::tasks
        .filter(task_dsl::owner.eq(user.ref_id()).or(task_dsl::id.eq_any(shared_task_ids)))
        .filter(task_dsl::completed.eq(false))
        .select(Task::as_select());

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<Task>(connection) {
        Ok(found_tasks) => Ok(found_tasks),
        Err(e) => Err(ApplicationError::DieselError(e)),
    }
}

pub fn get_new_auth_key(user: &User, given_password: &CleartextPassword, hashed_password: &argon2::PasswordHash, connection: &mut PgConnection) -> Result<Uuid, ApplicationError> {
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
                    Err(ApplicationError::DieselError(e))
                }

            }
        },
        Err(_) => {
            Err(ApplicationError::InvalidPassword)
        }
    }

}

pub fn get_task_by_id(task_id: &i32, connection: &mut PgConnection) -> Result<Task, ApplicationError> {
    use crate::schema::tasks::dsl::*;

    let query = tasks.filter(id.eq(task_id)).select(Task::as_select());

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<Task>(connection) {
        Ok(found_tasks) => {
            if !(found_tasks.is_empty()) {
                Ok(found_tasks.into_iter().next().unwrap())
            } else {
                Err(ApplicationError::DieselError(diesel::result::Error::NotFound))
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Query failed while getting task by id: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn get_user_by_name(un: &Username, connection: &mut PgConnection) -> Result<User, ApplicationError> {
    use crate::schema::users::dsl::*;

    let query = users
        .filter(username.eq(un.as_ref()))
        .select(User::as_select());
    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<User>(connection) {
        Ok(found_users) => {
            if !found_users.is_empty() {
                Ok(found_users.into_iter().next().unwrap())
            } else {
                Err(ApplicationError::DieselError(diesel::result::Error::NotFound))
            }
        },
        Err(e) => {
            event!(Level::ERROR, "Query failed while getting user by name: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn logout(auth_key: &String, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    use crate::schema::auth_keys::dsl::*;

    let query = diesel::delete(auth_keys.filter(key.eq(auth_key)));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.execute(connection) {
        Ok(_) => Ok(()),
        Err(e) => Err(ApplicationError::DieselError(e)),
    }

}

pub fn update_task(caller: &User, task_id: &i32, completed: Option<bool>, title: Option<TaskTitle>, description: Option<TaskDescription>, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    use crate::schema::tasks::dsl;

    // Owners and read-write share holders may update the task. Read-only
    // and unrelated users may not. A missing task and a task the caller
    // can't write both produce Unauthorized, so we don't leak which task
    // IDs exist.
    match get_task_permission(caller, task_id, connection) {
        Ok(Permission::Owner) | Ok(Permission::ReadWrite) => (),
        Ok(_) => return Err(ApplicationError::Unauthorized),
        Err(ApplicationError::DieselError(diesel::result::Error::NotFound)) => {
            return Err(ApplicationError::Unauthorized)
        },
        Err(e) => return Err(e),
    }

    let changeset = TaskUpdate {
        completed,
        title: title.map(|t| t.into()),
        description: description.map(|d| d.into()),
    };

    let update_query =
        diesel::update(dsl::tasks.filter(dsl::id.eq(task_id))).set(changeset);

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&update_query));

    match update_query.execute(connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// Verify that `owner` owns the task `task_id`, without revealing whether
/// the task exists. Both a missing task and a task owned by someone else
/// produce `ApplicationError::Unauthorized`.
fn require_task_owner(owner: &User, task_id: &i32, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    match get_task_by_id(task_id, connection) {
        Ok(task) => {
            if task.owner_id() == owner.ref_id() {
                Ok(())
            } else {
                Err(ApplicationError::Unauthorized)
            }
        },
        Err(ApplicationError::DieselError(diesel::result::Error::NotFound)) => {
            Err(ApplicationError::Unauthorized)
        },
        Err(e) => Err(e),
    }
}

/// Grant `target` access to the caller's task, or replace the existing
/// permission if a share row already exists (upsert semantics).
pub fn add_share_for_task(owner: &User, task_id: &i32, target: &Username, permission: SharePermission, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    require_task_owner(owner, task_id, connection)?;

    // Unknown target username: 404. That's acceptable because usernames
    // are public information, and sharing requires naming the target user.
    let target_user = get_user_by_name(target, connection)?;

    use crate::schema::task_shares::dsl::{self, task_shares};

    let query = diesel::insert_into(task_shares)
        .values((dsl::task_id.eq(task_id), dsl::user_id.eq(target_user.ref_id()), dsl::permission.eq(permission)))
        .on_conflict((dsl::task_id, dsl::user_id))
        .do_update()
        .set(dsl::permission.eq(permission));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.execute(connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// Remove the share row granting `target` access to the caller's task.
/// A missing share row on a task the caller owns is a 404.
pub fn remove_share(owner: &User, task_id: &i32, target: &Username, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    require_task_owner(owner, task_id, connection)?;

    let target_user = get_user_by_name(target, connection)?;

    use crate::schema::task_shares::dsl::{self, task_shares};

    let query = diesel::delete(task_shares)
        .filter(dsl::task_id.eq(task_id))
        .filter(dsl::user_id.eq(target_user.ref_id()));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.execute(connection) {
        Ok(0) => {
            event!(Level::ERROR, "No share row found to delete");
            Err(ApplicationError::DieselError(diesel::result::Error::NotFound))
        },
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        },
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// One row of the share listing for a task.
#[derive(Clone, Debug)]
pub struct TaskShareInfo {
    username: String,
    permission: SharePermission,
}

impl TaskShareInfo {
    pub fn username(&self) -> &String {
        &self.username
    }

    pub fn permission(&self) -> SharePermission {
        self.permission
    }
}

/// List all shares for a task owned by the caller.
pub fn list_shares_for_task(owner: &User, task_id: &i32, connection: &mut PgConnection) -> Result<Vec<TaskShareInfo>, ApplicationError> {
    require_task_owner(owner, task_id, connection)?;

    use crate::schema::task_shares::dsl as share_dsl;
    use crate::schema::users::dsl as user_dsl;

    let query = share_dsl::task_shares
        .inner_join(user_dsl::users)
        .filter(share_dsl::task_id.eq(task_id))
        .select((user_dsl::username, share_dsl::permission));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    match query.load::<(String, SharePermission)>(connection) {
        Ok(rows) => Ok(rows.into_iter().map(|(username, permission)| TaskShareInfo { username, permission }).collect()),
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// Returns true if a user with the given name exists, false otherwise.
pub fn user_exists(name: &String, connection: &mut PgConnection) -> Result<bool, ApplicationError> {
    use crate::schema::users::dsl::*;
    let query = users.filter(username.eq(name)).select(User::as_select());
    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    match query.load(connection) {
        Ok(collection) => Ok(!collection.is_empty()),
        Err(e) => Err(ApplicationError::DieselError(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_types::{TaskTitle, Username};

    fn make_user_and_task(connection: &mut PgConnection, label: &str) -> (User, crate::core::Task) {
        let username = Username::new(format!("perm_{}_{}", label, Uuid::new_v4())).unwrap();
        add_user(&username, &String::from("not-a-real-hash"), connection).unwrap();
        let user = get_user_by_name(&username, connection).unwrap();
        let title = TaskTitle::new(format!("permission test task {label}")).unwrap();
        let task = add_task(&user, &title, false, None, connection).unwrap();
        (user, task)
    }

    fn share_task(connection: &mut PgConnection, task: &crate::core::Task, user: &User, perm: SharePermission) {
        use crate::schema::task_shares::dsl::*;
        diesel::insert_into(task_shares)
            .values((task_id.eq(task.id()), user_id.eq(user.ref_id()), permission.eq(perm)))
            .execute(connection)
            .expect("Unable to insert task_shares row");
    }

    #[test]
    fn task_shares_insert_select_and_unique_constraint() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (user, task) = make_user_and_task(connection, "share_test");

        use crate::schema::task_shares::dsl::*;
        diesel::insert_into(task_shares)
            .values((task_id.eq(task.id()), user_id.eq(user.ref_id()), permission.eq(SharePermission::Read)))
            .execute(connection)
            .expect("Unable to insert task_shares row");

        let found: Vec<TaskShare> = task_shares
            .select(TaskShare::as_select())
            .filter(task_id.eq(task.id()))
            .load(connection)
            .expect("Unable to select task_shares rows");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].permission(), SharePermission::Read);

        // The UNIQUE (task_id, user_id) constraint must reject a duplicate pair
        let duplicate = diesel::insert_into(task_shares)
            .values((task_id.eq(task.id()), user_id.eq(user.ref_id()), permission.eq(SharePermission::ReadWrite)))
            .execute(connection);
        assert!(duplicate.is_err());
    }

    #[test]
    fn permission_is_owner_for_task_owner() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (user, task) = make_user_and_task(connection, "owner");
        let perm = get_task_permission(&user, task.id(), connection).unwrap();
        assert_eq!(perm, Permission::Owner);
    }

    #[test]
    fn permission_is_read_write_for_read_write_share() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (owner, task) = make_user_and_task(connection, "rw_owner");
        let (grantee, _) = make_user_and_task(connection, "rw_grantee");
        share_task(connection, &task, &grantee, SharePermission::ReadWrite);
        let perm = get_task_permission(&grantee, task.id(), connection).unwrap();
        assert_eq!(perm, Permission::ReadWrite);
        // The owner still reports Owner, not ReadWrite.
        let owner_perm = get_task_permission(&owner, task.id(), connection).unwrap();
        assert_eq!(owner_perm, Permission::Owner);
    }

    #[test]
    fn permission_is_read_only_for_read_share() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (_, task) = make_user_and_task(connection, "ro_owner");
        let (grantee, _) = make_user_and_task(connection, "ro_grantee");
        share_task(connection, &task, &grantee, SharePermission::Read);
        let perm = get_task_permission(&grantee, task.id(), connection).unwrap();
        assert_eq!(perm, Permission::ReadOnly);
    }

    #[test]
    fn permission_is_none_without_relationship() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (_, task) = make_user_and_task(connection, "none_owner");
        let (stranger, _) = make_user_and_task(connection, "none_stranger");
        let perm = get_task_permission(&stranger, task.id(), connection).unwrap();
        assert_eq!(perm, Permission::None);
    }
}
