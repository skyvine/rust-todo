use zeroize::ZeroizeOnDrop;

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

pub(crate) struct TaskTitle(String);

impl TaskTitle {
    pub(crate) fn new(title: String) -> Self {
        Self(title)
    }
}

impl AsRef<String> for TaskTitle {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

pub(crate) struct Username(String);

impl Username {
    pub fn new(username: String) -> Result<Self, String> {
        if username.trim().is_empty() {
            Err(String::from("Username is empty or contains only whitespace."))
        } else {
            Ok(Self(username))
        }
    }
}