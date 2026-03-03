pub trait Store {
    fn ping(&self) -> bool;
}
