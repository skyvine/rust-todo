use crate::domain_types::{CleartextPassword, TaskDescription, TaskTitle, Username};
use argon2::{Argon2, password_hash::PasswordVerifier};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::env;
use tracing::{Level, event};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug)]
pub enum ApplicationError {
    DieselError(diesel::result::Error),
    InvalidAuthKey,
    InvalidData(String),
    InvalidPassword,
    QueryFailed(String),
    /// No credentials reached the server, or the ones presented were never
    /// accepted. Answered with a 401, which asks the client to authenticate.
    Unauthorized,
    /// The caller was authenticated and still may not do this. Answered with a
    /// 403, which asks for nothing new: another auth key would fail the same
    /// way, so telling the client to get one is a lie.
    Forbidden,
    UserExists,
}

/// A complete entry from the auth_keys table in the database
#[derive(Queryable, Selectable, ZeroizeOnDrop)]
#[diesel(table_name = crate::schema::auth_keys)]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AuthKey {
    id: i32,
    user_id: i32,
    key: String,
}

/// A complete entry from the tasks table in the database
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Queryable, Selectable, Serialize)]
#[diesel(table_name = crate::schema::tasks)]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Task {
    id: i32,
    owner: i32,
    title: String,
    description: Option<String>,
    completed: bool,
}

#[allow(dead_code)]
impl Task {
    // Constructors
    pub fn from_json_object(object: &serde_json::Value) -> Result<Self, ApplicationError> {
        let id = match object["id"].as_i64() {
            Some(number) => number,
            None => {
                return Err(ApplicationError::InvalidData(format!(
                    "{} should be a number",
                    object["id"]
                )));
            }
        } as i32;

        let owner = match object["owner"].as_i64() {
            Some(number) => number,
            None => {
                return Err(ApplicationError::InvalidData(format!(
                    "{} should be a number",
                    object["owner"]
                )));
            }
        } as i32;

        let completed = match object["completed"].as_bool() {
            Some(b) => b,
            None => {
                return Err(ApplicationError::InvalidData(format!(
                    "{} should be a bool",
                    object["completed"]
                )));
            }
        };

        let title = String::from(match object["title"].as_str() {
            Some(s) => s,
            None => {
                return Err(ApplicationError::InvalidData(format!(
                    "{} should be a string",
                    object["title"]
                )));
            }
        });

        let description = match object["description"].as_str() {
            Some(s) => Some(String::from(s)),
            None => {
                if object["description"].is_null() {
                    None
                } else {
                    return Err(ApplicationError::InvalidData(format!(
                        "{} should be a string",
                        object["description"]
                    )));
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
    completed: Option<bool>,
    title: Option<String>,
    description: Option<String>,
}

/// The level of access granted by a task share, stored as text in `task_shares.permission`.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
    diesel::expression::AsExpression,
    diesel::deserialize::FromSqlRow,
)]
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
            other => Err(ApplicationError::InvalidData(format!(
                "Unknown share permission: {other}"
            ))),
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
        let text = <String as diesel::deserialize::FromSql<
            diesel::sql_types::Text,
            diesel::pg::Pg,
        >>::from_sql(bytes)?;
        SharePermission::from_stored_text(&text).map_err(|e| match e {
            ApplicationError::InvalidData(msg) => msg.into(),
            other => format!("{other:?}").into(),
        })
    }
}

impl diesel::serialize::ToSql<diesel::sql_types::Text, diesel::pg::Pg> for SharePermission {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, diesel::pg::Pg>,
    ) -> diesel::serialize::Result {
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
    id: i32,
    task_id: i32,
    user_id: i32,
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
/// exist, so callers can keep hiding task existence behind a uniform 403.
pub fn get_task_permission(
    user: &User,
    task_id: &i32,
    connection: &mut PgConnection,
) -> Result<Permission, ApplicationError> {
    let task = get_task_by_id(task_id, connection)?;

    if task.owner_id() == user.ref_id() {
        return Ok(Permission::Owner);
    }

    use crate::schema::task_shares::dsl::{self, task_shares};

    let query = task_shares
        .filter(dsl::task_id.eq(task_id))
        .filter(dsl::user_id.eq(user.ref_id()))
        .select(TaskShare::as_select());

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<TaskShare>(connection) {
        Ok(rows) => match rows.into_iter().next() {
            Some(share) => match share.permission() {
                SharePermission::Read => Ok(Permission::ReadOnly),
                SharePermission::ReadWrite => Ok(Permission::ReadWrite),
            },
            None => Ok(Permission::None),
        },
        Err(e) => {
            event!(
                Level::ERROR,
                "Query failed while looking up task share: {e}"
            );
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn add_task(
    owner: &User,
    title: &TaskTitle,
    completed: bool,
    description: Option<&TaskDescription>,
    connection: &mut PgConnection,
) -> Result<Task, ApplicationError> {
    use crate::schema::tasks::dsl;

    let query = diesel::insert_into(dsl::tasks).values((
        dsl::owner.eq(owner.ref_id()),
        dsl::title.eq(title.as_ref()),
        dsl::description.eq(description.map(AsRef::as_ref)),
        dsl::completed.eq(completed),
    ));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.get_result(connection) {
        Ok(task) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(task)
        }
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn add_user(
    un: &Username,
    hashed_password: &String,
    connection: &mut PgConnection,
) -> Result<(), ApplicationError> {
    use crate::schema::users::dsl::*;

    let query =
        diesel::insert_into(users).values((username.eq(un.as_ref()), password.eq(hashed_password)));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.execute(connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        }
        Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        )) => {
            event!(Level::ERROR, "Unable to add user: username already exists");
            Err(ApplicationError::UserExists)
        }
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn auth_key_to_user(
    auth_key: &str,
    connection: &mut PgConnection,
) -> Result<User, ApplicationError> {
    use crate::schema::auth_keys::dsl::*;
    use crate::schema::users::dsl::*;

    let query = auth_keys
        .inner_join(users)
        .filter(key.eq(auth_key))
        .filter(expiration.gt(diesel::dsl::now))
        .select((AuthKey::as_select(), User::as_select()));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<(AuthKey, User)>(connection) {
        Ok(results) => {
            if results.is_empty() {
                Err(ApplicationError::InvalidAuthKey)
            } else {
                Ok(results[0].1.clone())
            }
        }
        Err(e) => Err(ApplicationError::DieselError(e)),
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
            return Err(ConnectionError::InvalidConnectionUrl(msg));
        }
    };

    let connection = PgConnection::establish(&database_url);
    database_url.zeroize();
    connection
}

pub fn get_new_auth_key(
    user: &User,
    given_password: &CleartextPassword,
    hashed_password: &argon2::PasswordHash,
    connection: &mut PgConnection,
) -> Result<Uuid, ApplicationError> {
    use crate::schema::auth_keys::dsl::*;

    match Argon2::default().verify_password(given_password.as_ref().as_bytes(), hashed_password) {
        Ok(_) => {
            let new_key = Uuid::new_v4();
            let query = diesel::insert_into(auth_keys)
                .values((user_id.eq(user.ref_id()), key.eq(format!("{new_key}"))));
            event!(
                Level::TRACE,
                "Running query: {}",
                diesel::debug_query::<diesel::pg::Pg, _>(&query)
            );

            match query.execute(connection) {
                Ok(_) => Ok(new_key),
                Err(e) => {
                    event!(Level::ERROR, "Unable to insert new auth key: {e}");
                    Err(ApplicationError::DieselError(e))
                }
            }
        }
        Err(_) => Err(ApplicationError::InvalidPassword),
    }
}

pub fn get_task_by_id(
    task_id: &i32,
    connection: &mut PgConnection,
) -> Result<Task, ApplicationError> {
    use crate::schema::tasks::dsl::*;

    let query = tasks.filter(id.eq(task_id)).select(Task::as_select());

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<Task>(connection) {
        Ok(found_tasks) => {
            if !(found_tasks.is_empty()) {
                Ok(found_tasks.into_iter().next().unwrap())
            } else {
                Err(ApplicationError::DieselError(
                    diesel::result::Error::NotFound,
                ))
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Query failed while getting task by id: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// Every task the user can see: the tasks they own plus the tasks shared with
/// them, narrowed to those whose `completed` is the given value.
///
/// `completed` is the collection's filter, and `None` means no filter at all:
/// the whole collection, not an empty one. Both views are built by this one
/// query so that the rule for which tasks a caller can see lives in one place.
pub fn get_tasks_for_user(
    user: &User,
    completed: Option<bool>,
    connection: &mut PgConnection,
) -> Result<Vec<Task>, ApplicationError> {
    use crate::schema::task_shares::dsl as share_dsl;
    use crate::schema::tasks::dsl as task_dsl;

    // Tasks shared with the user, by id, as a subquery.
    let shared_task_ids = share_dsl::task_shares
        .filter(share_dsl::user_id.eq(user.ref_id()))
        .select(share_dsl::task_id);

    // Boxed so that the filter below can be added or left out without two
    // copies of the "owned or shared" subquery above.
    let mut query = task_dsl::tasks
        .filter(
            task_dsl::owner
                .eq(user.ref_id())
                .or(task_dsl::id.eq_any(shared_task_ids)),
        )
        .into_boxed::<diesel::pg::Pg>();

    if let Some(completed) = completed {
        query = query.filter(task_dsl::completed.eq(completed));
    }

    let query = query.select(Task::as_select());

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<Task>(connection) {
        Ok(found_tasks) => Ok(found_tasks),
        Err(e) => Err(ApplicationError::DieselError(e)),
    }
}

pub fn get_user_by_id(
    user_id: &i32,
    connection: &mut PgConnection,
) -> Result<User, ApplicationError> {
    use crate::schema::users::dsl::*;

    let query = users.filter(id.eq(user_id)).select(User::as_select());
    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<User>(connection) {
        Ok(found_users) => {
            if !found_users.is_empty() {
                Ok(found_users.into_iter().next().unwrap())
            } else {
                Err(ApplicationError::DieselError(
                    diesel::result::Error::NotFound,
                ))
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Query failed while getting user by id: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn get_user_by_name(
    un: &Username,
    connection: &mut PgConnection,
) -> Result<User, ApplicationError> {
    use crate::schema::users::dsl::*;

    let query = users
        .filter(username.eq(un.as_ref()))
        .select(User::as_select());
    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<User>(connection) {
        Ok(found_users) => {
            if !found_users.is_empty() {
                Ok(found_users.into_iter().next().unwrap())
            } else {
                Err(ApplicationError::DieselError(
                    diesel::result::Error::NotFound,
                ))
            }
        }
        Err(e) => {
            event!(Level::ERROR, "Query failed while getting user by name: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

pub fn logout(auth_key: &str, connection: &mut PgConnection) -> Result<(), ApplicationError> {
    use crate::schema::auth_keys::dsl::*;

    let query = diesel::delete(auth_keys.filter(key.eq(auth_key)));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.execute(connection) {
        Ok(_) => Ok(()),
        Err(e) => Err(ApplicationError::DieselError(e)),
    }
}

pub fn update_task(
    caller: &User,
    task_id: &i32,
    completed: Option<bool>,
    title: Option<TaskTitle>,
    description: Option<TaskDescription>,
    connection: &mut PgConnection,
) -> Result<(), ApplicationError> {
    use crate::schema::tasks::dsl;

    // Owners and read-write share holders may update the task. Read-only
    // and unrelated users may not. A missing task and a task the caller
    // can't write both produce Forbidden, so we don't leak which task
    // IDs exist.
    match get_task_permission(caller, task_id, connection) {
        Ok(Permission::Owner) | Ok(Permission::ReadWrite) => (),
        Ok(_) => return Err(ApplicationError::Forbidden),
        Err(ApplicationError::DieselError(diesel::result::Error::NotFound)) => {
            return Err(ApplicationError::Forbidden);
        }
        Err(e) => return Err(e),
    }

    let changeset = TaskUpdate {
        completed,
        title: title.map(|t| t.into()),
        description: description.map(|d| d.into()),
    };

    let update_query = diesel::update(dsl::tasks.filter(dsl::id.eq(task_id))).set(changeset);

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&update_query)
    );

    match update_query.execute(connection) {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        }
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// Verify that `owner` owns the task `task_id`, without revealing whether
/// the task exists. Both a missing task and a task owned by someone else
/// produce `ApplicationError::Forbidden`.
fn require_task_owner(
    owner: &User,
    task_id: &i32,
    connection: &mut PgConnection,
) -> Result<(), ApplicationError> {
    match get_task_by_id(task_id, connection) {
        Ok(task) => {
            if task.owner_id() == owner.ref_id() {
                Ok(())
            } else {
                Err(ApplicationError::Forbidden)
            }
        }
        Err(ApplicationError::DieselError(diesel::result::Error::NotFound)) => {
            Err(ApplicationError::Forbidden)
        }
        Err(e) => Err(e),
    }
}

/// What a write to a share's URI did.
///
/// A `PUT` on `/tasks/{id}/shares/{username}` is a single assignment which
/// either creates the share or replaces the permission on one which was
/// already there, and the two are different answers to the same request:
/// RFC 9110 §9.3.4 has a created resource answered with `201 Created` and a
/// `Location`, and a replaced one with `200 OK`. `Result<(), _>` cannot
/// report that, so the share itself and which case it was travel together.
#[derive(Clone, Debug)]
pub enum ShareWrite {
    Created(Share),
    Replaced(Share),
}

impl ShareWrite {
    /// The share as it stands after the write, whichever case it was.
    pub fn share(&self) -> &Share {
        match self {
            ShareWrite::Created(share) => share,
            ShareWrite::Replaced(share) => share,
        }
    }
}

/// Assign `permission` to `target` on the caller's task, at the share's own
/// URI: the share is created if it is not there, and its permission is
/// replaced if it is.
///
/// This is one statement on purpose. Which of the two happened has to be
/// answered from the write itself, and a write split into an insert and an
/// update leaves a window between them that another writer can slip through:
/// `ON CONFLICT DO NOTHING` deliberately takes no lock on the conflicting row,
/// so under `READ COMMITTED` a concurrent `DELETE` committed in that window
/// leaves the update matching nothing, and there is no honest answer left to
/// give. `DO UPDATE` closes the window instead of guarding it: it locks the
/// conflicting row until its transaction ends, so a concurrent write waits
/// rather than interleaving, and a conflict whose row turns out to have been
/// deleted becomes the insert it was before the conflict was found.
///
/// Whether the statement inserted or updated is read back out of the row it
/// returns. `xmax` names the transaction which last wrote a row, and is zero on
/// a row this statement inserted, because nothing has written it since. A share
/// which was already there with the permission this statement carries is still
/// reported as replaced, because the row was written again: it is the same
/// resource, not a new one.
pub fn add_share_for_task(
    owner: &User,
    task_id: &i32,
    target: &Username,
    permission: SharePermission,
    connection: &mut PgConnection,
) -> Result<ShareWrite, ApplicationError> {
    require_task_owner(owner, task_id, connection)?;

    // Unknown target username: 404. That's acceptable because usernames
    // are public information, and sharing requires naming the target user.
    let target_user = get_user_by_name(target, connection)?;

    use crate::schema::task_shares::dsl::{self, task_shares};

    let query = diesel::insert_into(task_shares)
        .values((
            dsl::task_id.eq(task_id),
            dsl::user_id.eq(target_user.ref_id()),
            dsl::permission.eq(permission),
        ))
        .on_conflict((dsl::task_id, dsl::user_id))
        .do_update()
        .set(dsl::permission.eq(permission))
        // `xmax` is a system column every table has, and it is the one thing in
        // the returned row which says whether this statement was the write that
        // made it.
        .returning((
            dsl::id,
            diesel::dsl::sql::<diesel::sql_types::Bool>("(xmax = 0)"),
        ));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.get_result::<(i32, bool)>(connection) {
        Ok((id, inserted)) => {
            event!(
                Level::TRACE,
                "Query succeeded: the share was {}",
                if inserted { "created" } else { "replaced" }
            );
            let share = Share {
                id,
                username: target_user.ref_username().clone(),
                permission,
            };
            Ok(if inserted {
                ShareWrite::Created(share)
            } else {
                ShareWrite::Replaced(share)
            })
        }
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// Remove the share row granting `target` access to the caller's task.
///
/// A share which is not there is not a failure: `DELETE` is idempotent, so
/// removing a share which is already gone asks for the state the caller
/// already has, and is answered the same way. (An unknown *username* is still
/// a 404, because that is a lookup of a user which does not exist rather than
/// a share which does.)
pub fn remove_share(
    owner: &User,
    task_id: &i32,
    target: &Username,
    connection: &mut PgConnection,
) -> Result<(), ApplicationError> {
    require_task_owner(owner, task_id, connection)?;

    let target_user = get_user_by_name(target, connection)?;

    use crate::schema::task_shares::dsl::{self, task_shares};

    let query = diesel::delete(task_shares)
        .filter(dsl::task_id.eq(task_id))
        .filter(dsl::user_id.eq(target_user.ref_id()));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.execute(connection) {
        Ok(0) => {
            event!(
                Level::TRACE,
                "No share row to delete: there was none, which is the state asked for"
            );
            Ok(())
        }
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            Ok(())
        }
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
    }
}

/// One share as the API represents it: the share row's own id, the user it is
/// for, and the permission it grants them.
///
/// This is what a client is shown for a share whether it came from the
/// collection or from a `PUT`, so that one share has one representation.
#[derive(Clone, Debug)]
pub struct Share {
    id: i32,
    username: String,
    permission: SharePermission,
}

impl Share {
    pub fn id(&self) -> &i32 {
        &self.id
    }

    pub fn username(&self) -> &String {
        &self.username
    }

    pub fn permission(&self) -> SharePermission {
        self.permission
    }
}

/// List all shares for a task owned by the caller.
pub fn list_shares_for_task(
    owner: &User,
    task_id: &i32,
    connection: &mut PgConnection,
) -> Result<Vec<Share>, ApplicationError> {
    require_task_owner(owner, task_id, connection)?;

    use crate::schema::task_shares::dsl as share_dsl;
    use crate::schema::users::dsl as user_dsl;

    let query = share_dsl::task_shares
        .inner_join(user_dsl::users)
        .filter(share_dsl::task_id.eq(task_id))
        .select((share_dsl::id, user_dsl::username, share_dsl::permission));

    event!(
        Level::TRACE,
        "Running query: {}",
        diesel::debug_query::<diesel::pg::Pg, _>(&query)
    );

    match query.load::<(i32, String, SharePermission)>(connection) {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(|(id, username, permission)| Share {
                id,
                username,
                permission,
            })
            .collect()),
        Err(e) => {
            event!(Level::ERROR, "Query Failed: {e}");
            Err(ApplicationError::DieselError(e))
        }
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

    fn share_task(
        connection: &mut PgConnection,
        task: &crate::core::Task,
        user: &User,
        perm: SharePermission,
    ) {
        use crate::schema::task_shares::dsl::*;
        diesel::insert_into(task_shares)
            .values((
                task_id.eq(task.id()),
                user_id.eq(user.ref_id()),
                permission.eq(perm),
            ))
            .execute(connection)
            .expect("Unable to insert task_shares row");
    }

    #[test]
    fn task_shares_insert_select_and_unique_constraint() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (user, task) = make_user_and_task(connection, "share_test");

        use crate::schema::task_shares::dsl::*;
        diesel::insert_into(task_shares)
            .values((
                task_id.eq(task.id()),
                user_id.eq(user.ref_id()),
                permission.eq(SharePermission::Read),
            ))
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
            .values((
                task_id.eq(task.id()),
                user_id.eq(user.ref_id()),
                permission.eq(SharePermission::ReadWrite),
            ))
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

    /// The caller of `add_share_for_task` has to tell a created share from a
    /// replaced one, because the status it answers with depends on which
    /// happened, and an upsert which reports nothing cannot be asked.
    #[test]
    fn add_share_reports_created_then_replaced() {
        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (owner, task) = make_user_and_task(connection, "share_write");
        let (grantee, _) = make_user_and_task(connection, "share_write_target");
        let target = Username::new(grantee.ref_username().clone()).unwrap();

        let created = add_share_for_task(
            &owner,
            task.id(),
            &target,
            SharePermission::Read,
            connection,
        )
        .unwrap();
        assert!(
            matches!(created, ShareWrite::Created(_)),
            "A share which was not there is created."
        );
        let created_share = created.share();
        assert_eq!(created_share.username(), grantee.ref_username());
        assert_eq!(created_share.permission(), SharePermission::Read);

        // The same permission again is a replacement, not a second share: the
        // request is idempotent, and only a row which is really there can be
        // reported as having been replaced.
        let replaced = add_share_for_task(
            &owner,
            task.id(),
            &target,
            SharePermission::ReadWrite,
            connection,
        )
        .unwrap();
        assert!(
            matches!(replaced, ShareWrite::Replaced(_)),
            "A share which was already there has its permission replaced."
        );
        assert_eq!(
            replaced.share().permission(),
            SharePermission::ReadWrite,
            "The replaced share reports the permission it now carries."
        );

        let shares = list_shares_for_task(&owner, task.id(), connection).unwrap();
        assert_eq!(
            shares.len(),
            1,
            "Writing twice must not add a second share."
        );
        assert_eq!(shares[0].id(), created_share.id());
        assert_eq!(shares[0].permission(), SharePermission::ReadWrite);

        // Removing a share which is not there leaves the requested state, so it
        // is a success rather than a lookup miss.
        remove_share(&owner, task.id(), &target, connection).unwrap();
        remove_share(&owner, task.id(), &target, connection)
            .expect("Removing an absent share is the state asked for, not a failure.");
        assert!(
            list_shares_for_task(&owner, task.id(), connection)
                .unwrap()
                .is_empty()
        );
    }

    /// The assignment is one statement, and this is the property that buys:
    /// concurrent writers queue on the row the conflict locks, so exactly one of
    /// them created the share, every other one replaced what that one created,
    /// and every one of them reports the same share.
    ///
    /// A write split into an insert and an update keeps the same result here,
    /// because the first insert simply wins. What it cannot promise is the
    /// answer to the harder question: its insert takes no lock, so a `DELETE`
    /// committed between the insert and the update leaves the update matching
    /// nothing, and there is no honest answer left to give.
    #[test]
    fn concurrent_share_writes_create_the_share_exactly_once() {
        const WRITERS: usize = 8;

        let connection = &mut establish_connection().expect("Unable to connect to database");
        let (owner, task) = make_user_and_task(connection, "concurrent_write");
        let (grantee, _) = make_user_and_task(connection, "concurrent_write_target");
        let task_id = *task.id();
        let grantee_username = grantee.ref_username().clone();

        let writers: Vec<_> = (0..WRITERS)
            .map(|_| {
                let owner = owner.clone();
                let grantee_username = grantee_username.clone();
                std::thread::spawn(move || {
                    let target = Username::new(grantee_username).unwrap();
                    let mut connection =
                        establish_connection().expect("Unable to connect to database");
                    add_share_for_task(
                        &owner,
                        &task_id,
                        &target,
                        SharePermission::Read,
                        &mut connection,
                    )
                })
            })
            .collect();
        let writes: Vec<_> = writers
            .into_iter()
            .map(|writer| writer.join().expect("A writer thread panicked"))
            .collect();

        let created: Vec<&Share> = writes
            .iter()
            .filter_map(|write| match write {
                Ok(ShareWrite::Created(share)) => Some(share),
                _ => None,
            })
            .collect();
        assert_eq!(
            created.len(),
            1,
            "Exactly one concurrent writer can have created the share: {writes:?}"
        );
        assert!(
            writes.iter().all(|write| write.is_ok()),
            "No concurrent writer may be left with nothing to report: {writes:?}"
        );

        let reported: Vec<i32> = writes
            .iter()
            .map(|write| *write.as_ref().unwrap().share().id())
            .collect();
        assert!(
            reported.iter().all(|id| *id == reported[0]),
            "Every writer must report the one share, not a second one: {writes:?}"
        );

        let connection = &mut establish_connection().expect("Unable to connect to database");
        let shares = list_shares_for_task(&owner, &task_id, connection).unwrap();
        assert_eq!(
            shares.len(),
            1,
            "Eight concurrent writers must leave one share: {shares:?}"
        );
    }
}
