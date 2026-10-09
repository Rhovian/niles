use super::{Buckets, SessionState, Usage};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{env, fs};

const SESSION: &str = "
    SELECT input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
        estimated_cost_usd, last_activity_at, ended_at IS NOT NULL AS ended,
        (SELECT role = 'assistant' AND tool_calls IS NULL FROM messages
         WHERE session_id = s.id AND active = 1 ORDER BY id DESC LIMIT 1) AS replied
    FROM sessions s
    WHERE source = ?1 AND json_extract(model_config, '$._delegate_from') IS NULL
    ORDER BY started_at DESC LIMIT 1";

fn hermes_home(home: &Utf8Path, configured: Option<&str>) -> Result<Utf8PathBuf> {
    if let Some(path) = configured.map(str::trim).filter(|path| !path.is_empty()) {
        return Ok(match path.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if path == "~" => home.to_owned(),
            None => Utf8PathBuf::from(path),
        });
    }
    let root = home.join(".hermes");
    let active = root.join("active_profile");
    if active.exists() {
        let profile = fs::read_to_string(&active)
            .with_context(|| format!("read Hermes active profile {active}"))?;
        let profile = profile.trim();
        if !profile.is_empty() && profile != "default" {
            return Ok(root.join("profiles").join(profile));
        }
    }
    Ok(root)
}

pub(super) fn read_hermes(home: &Utf8Path, source: &str) -> Result<Option<Usage>> {
    let configured = match env::var("HERMES_HOME") {
        Ok(path) => Some(path),
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("HERMES_HOME is invalid"),
    };
    let path = hermes_home(home, configured.as_deref())?.join("state.db");
    let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open Hermes database {path}"))?;
    read_session(&db, source)
}

fn read_session(db: &Connection, source: &str) -> Result<Option<Usage>> {
    db.query_row(SESSION, params![source], |row| {
        let ended: bool = row.get("ended")?;
        let replied: Option<bool> = row.get("replied")?;
        let last_activity_at: Option<f64> = row.get("last_activity_at")?;
        let last_turn_at = last_activity_at.and_then(|seconds| {
            DateTime::from_timestamp_micros((seconds * 1_000_000.0).round() as i64)
        });
        let mut usage = Usage {
            input_tokens: row.get("input_tokens")?,
            output_tokens: row.get("output_tokens")?,
            cache_read_tokens: row.get("cache_read_tokens")?,
            cache_write_tokens: Some(row.get("cache_write_tokens")?),
            reasoning_tokens: Some(row.get("reasoning_tokens")?),
            estimated_cost_usd: row.get("estimated_cost_usd")?,
            last_turn_at,
            state: if ended {
                Some(SessionState::Waiting)
            } else {
                replied.map(|waiting| {
                    if waiting {
                        SessionState::Waiting
                    } else {
                        SessionState::Working
                    }
                })
            },
            buckets: Buckets::default(),
            prompt_tokens: None,
            context_window: None,
        };
        // Hermes keeps only per-session totals, so the whole session lands in its last-activity
        // bucket, and it has no per-turn prompt to measure context by.
        if let Some(at) = last_turn_at {
            usage.buckets.add(at, usage.total_tokens());
        }
        Ok(usage)
    })
    .optional()
    .context("query Hermes session")
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The fixture's main session, which has activity and so lands in one bucket.
    pub(in crate::telemetry) fn fixture_usage() -> Usage {
        read_session(&fixture(), "niles:test").unwrap().unwrap()
    }

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            r#"CREATE TABLE sessions (id TEXT, source TEXT, started_at REAL,
            model_config TEXT, input_tokens INTEGER, output_tokens INTEGER,
            cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
            estimated_cost_usd REAL, last_activity_at REAL, ended_at REAL);
            CREATE TABLE messages (id INTEGER, session_id TEXT, active INTEGER,
            role TEXT, tool_calls TEXT);
            INSERT INTO sessions VALUES
            ('old', 'niles:test', 1, NULL, 1, 2, 3, 4, 5, NULL, NULL, NULL),
            ('main', 'niles:test', 2, NULL, 11, 22, 33, 44, 55,
             0.03, 1790694525.47223, NULL),
            ('delegate', 'niles:test', 3, '{"_delegate_from":"main"}',
             91, 92, 93, 94, 95, NULL, NULL, NULL);
            INSERT INTO messages VALUES (1, 'main', 1, 'assistant', NULL);"#,
        )
        .unwrap();
        db
    }

    #[test]
    fn selects_latest_non_delegate_with_all_counters_and_epoch_time() {
        let db = fixture();
        assert!(read_session(&db, "missing").unwrap().is_none());
        let usage = read_session(&db, "niles:test").unwrap().unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
                usage.reasoning_tokens
            ),
            (11, 22, 33, Some(44), Some(55))
        );
        assert_eq!(usage.estimated_cost_usd, Some(0.03));
        assert_eq!(
            usage.last_turn_at.unwrap().timestamp_micros(),
            1790694525472230
        );
    }

    #[test]
    fn waiting_working_and_ended_states() {
        let db = fixture();
        assert_eq!(
            read_session(&db, "niles:test").unwrap().unwrap().state,
            Some(SessionState::Waiting)
        );
        db.execute("UPDATE messages SET tool_calls = '[{}]' WHERE id = 1", [])
            .unwrap();
        assert_eq!(
            read_session(&db, "niles:test").unwrap().unwrap().state,
            Some(SessionState::Working)
        );
        db.execute("UPDATE sessions SET ended_at = 4 WHERE id = 'main'", [])
            .unwrap();
        assert_eq!(
            read_session(&db, "niles:test").unwrap().unwrap().state,
            Some(SessionState::Waiting)
        );
    }

    #[test]
    fn no_active_message_has_no_state_or_activity_time() {
        let db = fixture();
        db.execute_batch(
            "UPDATE messages SET active = 0;
            UPDATE sessions SET last_activity_at = NULL WHERE id = 'main';",
        )
        .unwrap();
        let usage = read_session(&db, "niles:test").unwrap().unwrap();
        assert_eq!(usage.state, None);
        assert_eq!(usage.last_turn_at, None);
    }

    #[test]
    fn resolves_configured_home_and_active_profile() {
        let home = crate::test_support::temp_test_path("hermes-home");
        let root = home.join(".hermes");
        fs::create_dir_all(&root).unwrap();
        assert_eq!(
            hermes_home(&home, Some(" ~/custom ")).unwrap(),
            home.join("custom")
        );
        assert_eq!(hermes_home(&home, Some("~")).unwrap(), home);
        assert_eq!(hermes_home(&home, None).unwrap(), root);
        fs::write(root.join("active_profile"), " team \n").unwrap();
        assert_eq!(
            hermes_home(&home, Some(" ")).unwrap(),
            root.join("profiles/team")
        );
        fs::write(root.join("active_profile"), " default \n").unwrap();
        assert_eq!(hermes_home(&home, None).unwrap(), root);
        fs::remove_dir_all(&home).unwrap();
    }
}
