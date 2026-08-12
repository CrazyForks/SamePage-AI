use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::app_paths::{CONVERSATIONS_DIR_NAME, RUNS_DIR_NAME};

const CONVERSATION_INDEX_FILE_NAME: &str = "conversation_index.jsonl";
const RUN_INDEX_FILE_NAME: &str = "run_index.jsonl";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalLogTimestamp {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl LocalLogTimestamp {
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    pub fn now_utc() -> Self {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);

        Self::from_unix_seconds(unix_seconds)
    }

    pub fn from_unix_seconds(unix_seconds: i64) -> Self {
        let days = unix_seconds.div_euclid(86_400);
        let seconds_of_day = unix_seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);

        Self {
            year: year as u16,
            month: month as u8,
            day: day as u8,
            hour: (seconds_of_day / 3_600) as u8,
            minute: ((seconds_of_day % 3_600) / 60) as u8,
            second: (seconds_of_day % 60) as u8,
        }
    }

    pub fn to_rfc3339_millis(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    fn date_bucket(self) -> String {
        format!("{:04}/{:02}/{:02}", self.year, self.month, self.day)
    }

    fn file_timestamp(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_unix_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year, month, day)
}

pub fn conversation_index_path(buddy_home: &Path) -> PathBuf {
    buddy_home.join(CONVERSATION_INDEX_FILE_NAME)
}

pub fn run_index_path(buddy_home: &Path) -> PathBuf {
    buddy_home.join(RUN_INDEX_FILE_NAME)
}

pub fn conversation_log_path(
    buddy_home: &Path,
    timestamp: LocalLogTimestamp,
    conversation_id: &str,
) -> PathBuf {
    buddy_home
        .join(CONVERSATIONS_DIR_NAME)
        .join(timestamp.date_bucket())
        .join(format!(
            "conversation-{}-{}.jsonl",
            timestamp.file_timestamp(),
            conversation_id
        ))
}

pub fn run_log_path(buddy_home: &Path, timestamp: LocalLogTimestamp, run_id: &str) -> PathBuf {
    buddy_home
        .join(RUNS_DIR_NAME)
        .join(timestamp.date_bucket())
        .join(format!(
            "run-{}-{}.jsonl",
            timestamp.file_timestamp(),
            run_id
        ))
}

pub fn parse_rfc3339_utc_seconds(timestamp: &str) -> Option<u64> {
    let year = parse_u32(timestamp.get(0..4)?)? as i32;
    let month = parse_u32(timestamp.get(5..7)?)?;
    let day = parse_u32(timestamp.get(8..10)?)?;
    let hour = parse_u32(timestamp.get(11..13)?)?;
    let minute = parse_u32(timestamp.get(14..16)?)?;
    let second = parse_u32(timestamp.get(17..19)?)?;

    if timestamp.get(4..5) != Some("-")
        || timestamp.get(7..8) != Some("-")
        || timestamp.get(10..11) != Some("T")
        || timestamp.get(13..14) != Some(":")
        || timestamp.get(16..17) != Some(":")
        || !timestamp.ends_with('Z')
        || month == 0
        || month > 12
        || day == 0
        || day > 31
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    u64::try_from(days)
        .ok()?
        .checked_mul(24 * 60 * 60)?
        .checked_add(u64::from(hour) * 60 * 60)?
        .checked_add(u64::from(minute) * 60)?
        .checked_add(u64::from(second))
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i32::try_from(month).ok()?;
    let day = i32::try_from(day).ok()?;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    Some(i64::from(era) * 146_097 + i64::from(day_of_era) - 719_468)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        conversation_index_path, conversation_log_path, parse_rfc3339_utc_seconds, run_index_path,
        run_log_path, LocalLogTimestamp,
    };

    #[test]
    fn builds_date_bucketed_conversation_log_path() {
        let timestamp = LocalLogTimestamp::new(2026, 7, 6, 9, 8, 7);
        let path = conversation_log_path(
            &PathBuf::from("/tmp/lexora-buddy"),
            timestamp,
            "conversation-1",
        );

        assert_eq!(
            path,
            PathBuf::from(
                "/tmp/lexora-buddy/conversations/2026/07/06/conversation-2026-07-06T09-08-07-conversation-1.jsonl"
            )
        );
    }

    #[test]
    fn builds_date_bucketed_run_log_path() {
        let timestamp = LocalLogTimestamp::new(2026, 7, 6, 9, 8, 7);
        let path = run_log_path(&PathBuf::from("/tmp/lexora-buddy"), timestamp, "run-1");

        assert_eq!(
            path,
            PathBuf::from("/tmp/lexora-buddy/runs/2026/07/06/run-2026-07-06T09-08-07-run-1.jsonl")
        );
    }

    #[test]
    fn builds_conversation_index_path() {
        assert_eq!(
            conversation_index_path(&PathBuf::from("/tmp/lexora-buddy")),
            PathBuf::from("/tmp/lexora-buddy/conversation_index.jsonl")
        );
    }

    #[test]
    fn builds_run_index_path() {
        assert_eq!(
            run_index_path(&PathBuf::from("/tmp/lexora-buddy")),
            PathBuf::from("/tmp/lexora-buddy/run_index.jsonl")
        );
    }

    #[test]
    fn converts_unix_seconds_to_utc_timestamp() {
        let timestamp = LocalLogTimestamp::from_unix_seconds(1_783_328_887);

        assert_eq!(timestamp.to_rfc3339_millis(), "2026-07-06T09:08:07.000Z");
        assert_eq!(timestamp.date_bucket(), "2026/07/06");
        assert_eq!(timestamp.file_timestamp(), "2026-07-06T09-08-07");
    }

    #[test]
    fn parses_rfc3339_utc_seconds_with_optional_millis() {
        assert_eq!(parse_rfc3339_utc_seconds("1970-01-01T00:00:01Z"), Some(1));
        assert_eq!(
            parse_rfc3339_utc_seconds("1970-01-02T00:00:00.000Z"),
            Some(86_400)
        );
        assert_eq!(parse_rfc3339_utc_seconds("1970-01-01 00:00:01Z"), None);
    }
}
