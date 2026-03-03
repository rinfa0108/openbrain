#[cfg(test)]
mod tests {
    use openbrain_core::SPEC_VERSION;

    #[test]
    fn workspace_wires_up() {
        assert_eq!(SPEC_VERSION, "0.1");
        assert_eq!(2 + 2, 4);
    }
}
