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

impl AsRef<String> for Username {
    fn as_ref(&self) -> &String {
        &self.0
    }
}
