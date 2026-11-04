//! Central command outcome to process exit-code mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Error,
    Usage,
    Damage,
    Locked,
}

pub const fn code(result: Result<(), Failure>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(Failure::Error) => 1,
        Err(Failure::Usage) => 2,
        Err(Failure::Damage) => 3,
        Err(Failure::Locked) => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_all_outcomes() {
        assert_eq!(code(Ok(())), 0);
        for (failure, expected) in [
            (Failure::Error, 1),
            (Failure::Usage, 2),
            (Failure::Damage, 3),
            (Failure::Locked, 4),
        ] {
            assert_eq!(code(Err(failure)), expected);
        }
    }
}
