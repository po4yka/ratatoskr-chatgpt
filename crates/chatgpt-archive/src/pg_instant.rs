//! Moves `PostgreSQL` instants onto the wire without a time-library dependency.
//!
//! The database renders an instant in UTC with microsecond precision, and the wire
//! admits exactly one spelling per instant (no trailing zeros in the fraction), so a
//! rendering is canonicalized before it is parsed.

use ratatoskr_identifiers::WireTimestamp;

/// SQL that renders `expression` as a UTC instant with a microsecond fraction.
pub(crate) fn pg_instant_sql(expression: &str) -> String {
    format!("to_char({expression} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US')")
}

/// The canonical wire instant of a [`pg_instant_sql`] rendering.
pub(crate) fn wire_instant(rendered: &str) -> Option<WireTimestamp> {
    let (whole, fraction) = rendered.split_once('.').unwrap_or((rendered, ""));
    let fraction = fraction.trim_end_matches('0');
    let spelled = if fraction.is_empty() {
        format!("{whole}Z")
    } else {
        format!("{whole}.{fraction}Z")
    };
    WireTimestamp::parse(&spelled).ok()
}

/// The database clock now, as a wire instant. `now()` is fixed for a transaction,
/// so every read inside one transaction agrees.
pub(crate) async fn database_now(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<WireTimestamp, sqlx::Error> {
    let rendered: String = sqlx::query_scalar(&format!("SELECT {}", pg_instant_sql("now()")))
        .fetch_one(&mut **transaction)
        .await?;
    wire_instant(&rendered).ok_or_else(|| {
        sqlx::Error::Decode("the database clock rendered a non-canonical instant".into())
    })
}

#[cfg(test)]
mod tests {
    use super::wire_instant;

    #[test]
    fn trailing_zeros_of_the_fraction_are_trimmed() {
        assert_eq!(
            wire_instant("2026-10-10T12:00:00.100000")
                .map(ratatoskr_identifiers::WireTimestamp::to_wire),
            Some("2026-10-10T12:00:00.1Z".to_owned())
        );
    }

    #[test]
    fn a_whole_second_has_no_fraction() {
        assert_eq!(
            wire_instant("2026-10-10T12:00:00.000000")
                .map(ratatoskr_identifiers::WireTimestamp::to_wire),
            Some("2026-10-10T12:00:00Z".to_owned())
        );
    }

    #[test]
    fn a_non_instant_has_no_wire_form() {
        assert_eq!(wire_instant("not an instant"), None);
    }
}
