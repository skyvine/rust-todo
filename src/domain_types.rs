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
