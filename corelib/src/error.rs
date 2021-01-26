pub trait Error {
    fn code(&self) -> i32;
    fn msg(&self) -> &str;
}

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;