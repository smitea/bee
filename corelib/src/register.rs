use crate::Result;

pub trait Register {
    fn inject_func(&self, ) -> Result<()>;
}