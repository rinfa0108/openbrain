pub const SPEC_VERSION: &str = "0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectType {
    Entity,
    Relation,
    Claim,
    Decision,
    Task,
    Artifact,
    ThoughtSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectStatus {
    Draft,
    Candidate,
    Canonical,
    Deprecated,
    Superseded,
}
