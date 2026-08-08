pub mod humble;
pub mod kindle;
pub mod manning;
pub mod manual;
pub mod packt;

pub trait Source {
    fn name(&self) -> &'static str;
    fn fetch(&self) -> anyhow::Result<Vec<crate::model::Book>>;
}
