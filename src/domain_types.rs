use zeroize::ZeroizeOnDrop;

/// A user's secret auth key, taken from the `Authorization` header.
///
/// The key lives in a newtype so that it is zeroized as soon as the request is
/// done with it, matching the `ZeroizeOnDrop` payload structs.
#[derive(ZeroizeOnDrop)]
pub struct AuthKey(String);

#[allow(dead_code)]
impl AuthKey {
    /// The key itself, for handing to [`crate::core::auth_key_to_user`].
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn new(key: String) -> Self {
        Self(key)
    }
}

/// A username and password pair, taken from an HTTP Basic `Authorization`
/// header.
///
/// Both halves are secrets, so the whole struct is zeroized as soon as the
/// request is done with it.
#[derive(ZeroizeOnDrop)]
pub(crate) struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    pub(crate) fn new(username: String, password: String) -> Self {
        Self { username, password }
    }

    /// Hands the username and the password over to the handler.
    ///
    /// The fields are emptied rather than moved out, so that the zeroizing
    /// `Drop` impl still has a well defined struct to work on.
    pub(crate) fn into_parts(mut self) -> (String, CleartextPassword) {
        (
            std::mem::take(&mut self.username),
            CleartextPassword::new(std::mem::take(&mut self.password)),
        )
    }
}

#[derive(ZeroizeOnDrop)]
pub(crate) struct CleartextPassword(String);

impl CleartextPassword {
    pub fn new(password: String) -> Self {
        Self(password)
    }
}

impl AsRef<String> for CleartextPassword {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl AsRef<String> for Username {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

pub(crate) struct TaskDescription(String);

impl TaskDescription {
    pub(crate) fn new(description: String) -> Self {
        Self(description)
    }
}

impl AsRef<String> for TaskDescription {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl From<TaskDescription> for String {
    fn from(value: TaskDescription) -> Self {
        value.0
    }
}

pub(crate) struct TaskTitle(String);

impl TaskTitle {
    pub(crate) fn new(title: String) -> Result<Self, String> {
        if title.trim().is_empty() {
            Err(String::from("Title is empty or contains only whitespace."))
        } else {
            Ok(Self(title))
        }
    }
}

impl AsRef<String> for TaskTitle {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl From<TaskTitle> for String {
    fn from(value: TaskTitle) -> Self {
        value.0
    }
}
pub(crate) struct Username(String);

impl Username {
    pub fn new(username: String) -> Result<Self, String> {
        if username.trim().is_empty() {
            Err(String::from(
                "Username is empty or contains only whitespace.",
            ))
        } else {
            Ok(Self(username))
        }
    }
}
