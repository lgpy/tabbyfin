//! The server's clock, estimated NTP-style from `GET /GetUtcTime`. SyncPlay schedules commands
//! in server time, so members whose clocks disagree still act together.

use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::client::Query;
use crate::{Client, Error, Result};

/// Samples kept; the one with the shortest round trip wins.
const SAMPLES: usize = 8;

/// One `/GetUtcTime` exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// Server clock minus local clock, in microseconds.
    pub offset_us: i64,
    /// Network time, without the server's processing time.
    pub round_trip: Duration,
}

impl ClockSample {
    /// From local send/receive times and the server's receive/transmit times, all in
    /// microseconds since the Unix epoch.
    fn from_times(sent: i64, server_received: i64, server_sent: i64, received: i64) -> Self {
        let offset_us = ((server_received - sent) + (server_sent - received)) / 2;
        let round_trip = (received - sent) - (server_sent - server_received);
        Self {
            offset_us,
            round_trip: Duration::from_micros(round_trip.max(0) as u64),
        }
    }
}

/// Recent clock samples. Add one every minute or so; a new member should take a few at once.
#[derive(Debug, Clone, Default)]
pub struct ServerClock {
    samples: VecDeque<ClockSample>,
}

impl ServerClock {
    pub fn add(&mut self, sample: ClockSample) {
        if self.samples.len() == SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// The most trustworthy sample: the one least affected by network delay.
    pub fn best(&self) -> Option<ClockSample> {
        self.samples.iter().min_by_key(|s| s.round_trip).copied()
    }

    /// A server timestamp as a local one. Unchanged until there's a sample.
    pub fn to_local(&self, server: SystemTime) -> SystemTime {
        shift(server, -self.best().map_or(0, |s| s.offset_us))
    }

    /// The server's clock now.
    pub fn server_now(&self) -> SystemTime {
        shift(SystemTime::now(), self.best().map_or(0, |s| s.offset_us))
    }
}

fn shift(time: SystemTime, by_us: i64) -> SystemTime {
    if by_us >= 0 {
        time + Duration::from_micros(by_us as u64)
    } else {
        time - Duration::from_micros(by_us.unsigned_abs())
    }
}

fn micros(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_micros() as i64,
        Err(before) => -(before.duration().as_micros() as i64),
    }
}

/// Parses the server's UTC timestamps, e.g. `2026-10-14T19:00:00.1234567Z`.
pub fn parse_utc(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let text = text
        .strip_suffix('Z')
        .or_else(|| text.strip_suffix("+00:00"))
        .unwrap_or(text);
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let (hms, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut hms = hms.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (hour, minute, second) = (hms.next()??, hms.next()??, hms.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Fractional seconds, to the microsecond.
    let digits: String = fraction.chars().take(6).collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let fraction_us = if digits.is_empty() {
        0
    } else {
        digits.parse::<i64>().ok()? * 10_i64.pow(6 - digits.len() as u32)
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some(shift(UNIX_EPOCH, seconds * 1_000_000 + fraction_us))
}

/// Formats a time the way the server expects, e.g. `2026-10-14T19:00:00.123456Z`.
pub fn format_utc(time: SystemTime) -> String {
    let us = micros(time);
    let (seconds, fraction) = (us.div_euclid(1_000_000), us.rem_euclid(1_000_000));
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:06}Z",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60
    )
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

impl Client {
    /// Takes one clock sample from the server.
    pub async fn clock_sample(&self) -> Result<ClockSample> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct UtcTime {
            request_reception_time: String,
            response_transmission_time: String,
        }
        let sent = SystemTime::now();
        let times: UtcTime = self.get("/GetUtcTime", &Query::new()).await?;
        let received = SystemTime::now();
        let parse = |text: &str| {
            parse_utc(text).ok_or_else(|| Error::Status {
                status: 200,
                message: format!("unreadable server time {text:?}"),
            })
        };
        Ok(ClockSample::from_times(
            micros(sent),
            micros(parse(&times.request_reception_time)?),
            micros(parse(&times.response_transmission_time)?),
            micros(received),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{ClockSample, ServerClock, format_utc, micros, parse_utc};

    #[test]
    fn parses_server_times() {
        let at = |text| parse_utc(text).map(micros);
        assert_eq!(at("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(at("2000-03-01T00:00:00Z"), Some(951_868_800_000_000));
        // .NET writes seven fractional digits; microseconds are kept.
        assert_eq!(at("2026-10-14T19:00:00.1234567Z"), Some(1_792_004_400_123_456));
        assert_eq!(at("2026-10-14T19:00:00.5+00:00"), Some(1_792_004_400_500_000));
        assert_eq!(at("2026-10-14T19:00:00"), Some(1_792_004_400_000_000));
        assert_eq!(at("2026-13-14T19:00:00Z"), None);
        assert_eq!(at("yesterday"), None);
    }

    #[test]
    fn formats_and_parses_back() {
        for text in [
            "1970-01-01T00:00:00.000000Z",
            "2000-02-29T23:59:59.999999Z",
            "2026-10-14T19:00:00.123456Z",
        ] {
            assert_eq!(format_utc(parse_utc(text).unwrap()), text);
        }
    }

    #[test]
    fn offset_and_round_trip() {
        // Local clock 2 s behind the server; 100 ms each way; 10 ms server processing.
        let sample = ClockSample::from_times(0, 2_100_000, 2_110_000, 210_000);
        assert_eq!(sample.offset_us, 2_000_000);
        assert_eq!(sample.round_trip, Duration::from_millis(200));
    }

    #[test]
    fn keeps_the_fastest_sample() {
        let mut clock = ServerClock::default();
        clock.add(ClockSample {
            offset_us: 5_000,
            round_trip: Duration::from_millis(80),
        });
        clock.add(ClockSample {
            offset_us: 1_000,
            round_trip: Duration::from_millis(10),
        });
        clock.add(ClockSample {
            offset_us: 9_000,
            round_trip: Duration::from_millis(300),
        });
        assert_eq!(clock.best().map(|s| s.offset_us), Some(1_000));
        assert_eq!(micros(clock.to_local(UNIX_EPOCH + Duration::from_secs(1))), 999_000);
    }
}
