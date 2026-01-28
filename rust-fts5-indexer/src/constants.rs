/// `SQLite` `application_id` used to identify ffts-grep databases.
///
/// This PRAGMA value is stored in the database header and is used to verify
/// that a database file was created by ffts-grep. The value 0xA17E6D42 is
/// unique to this project and prevents accidental use of foreign databases.
///
/// # References
/// - `SQLite` PRAGMA `application_id`: <https://www.sqlite.org/pragma.html#pragma_application_id>
pub const EXPECTED_APPLICATION_ID: u32 = 0xA17E_6D42;

/// `application_id` stored as i32 with the same bit pattern.
pub const APPLICATION_ID_I32: i32 = i32::from_ne_bytes(EXPECTED_APPLICATION_ID.to_ne_bytes());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_application_id_roundtrip() {
        let roundtrip = u32::from_ne_bytes(APPLICATION_ID_I32.to_ne_bytes());
        assert_eq!(roundtrip, EXPECTED_APPLICATION_ID);
    }
}
