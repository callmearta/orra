//! Aggregates over the dictation history, for the Insights page.
//!
//! Everything here is a pure fold over `&[Entry]` so the arithmetic that is easy
//! to get subtly wrong — streaks, the words-per-minute denominator, month
//! boundaries — is testable without a running app or a microphone. The frontend
//! only lays out what this returns.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use serde::Serialize;

use crate::state::Entry;

/// Sustained typing speed, for the "typing time saved" figure.
const TYPING_WPM: f64 = 40.0;
/// A mid-length novel, for the milestone line.
const WORDS_PER_BOOK: usize = 100_000;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub words: usize,
    pub dictations: usize,
    /// Distinct applications dictated into.
    pub apps: usize,
    /// Mean words per minute over the dictations that recorded a duration.
    pub wpm: Option<f64>,
    /// How many dictations carried a duration, so the UI can say what the mean
    /// covers instead of implying it is all of them.
    pub measured: usize,
    pub fixes: usize,
    pub fillers: usize,
    /// Mean Deepgram confidence, 0.0–1.0, over the dictations that reported one.
    pub confidence: Option<f64>,
    /// Distinct lowercased words across every transcript.
    pub vocabulary: usize,
    pub words_this_month: usize,
    pub words_last_month: usize,
    /// Minutes of typing the dictation saved at `TYPING_WPM`.
    pub minutes_saved: usize,
    pub books: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayCount {
    /// Local calendar day, `YYYY-MM-DD`.
    pub day: String,
    pub words: usize,
    pub dictations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppCount {
    /// The raw window class, e.g. `google-chrome-beta`.
    pub app: String,
    pub words: usize,
    pub dictations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Insights {
    pub totals: Totals,
    /// Every day with at least one dictation, oldest first.
    pub days: Vec<DayCount>,
    /// Busiest first.
    pub apps: Vec<AppCount>,
    pub current_streak: usize,
    pub longest_streak: usize,
}

/// The local calendar day an entry belongs to, or `None` for a timestamp too
/// far out of range to convert.
fn local_day(at_ms: u64) -> Option<NaiveDate> {
    Local
        .timestamp_millis_opt(at_ms as i64)
        .single()
        .map(|dt| dt.date_naive())
}

/// Words in a transcript, split on anything that is not a letter or digit, so
/// "well-known" counts once and punctuation never becomes a word of its own.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty())
}

pub fn insights(entries: &[Entry], today: NaiveDate) -> Insights {
    let mut totals = Totals {
        dictations: entries.len(),
        ..Default::default()
    };
    let mut per_day: HashMap<NaiveDate, (usize, usize)> = HashMap::new();
    let mut per_app: HashMap<&str, (usize, usize)> = HashMap::new();
    let mut vocabulary: HashSet<String> = HashSet::new();

    // Words spoken per minute can only be averaged over dictations that recorded
    // a duration; folding the untimed ones in would count their words against
    // someone else's seconds.
    let (mut timed_words, mut timed_ms) = (0usize, 0u64);
    let (mut conf_sum, mut conf_n) = (0f64, 0usize);

    // The 1st of this month and of the month before it. Both are derived from a
    // real calendar day, so neither can fail in practice, but a `None` here just
    // means the month totals stay at zero rather than that anything panics.
    let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1);
    let prev_month = this_month
        .and_then(|d| d.pred_opt())
        .and_then(|d| NaiveDate::from_ymd_opt(d.year(), d.month(), 1));

    for e in entries {
        totals.words += e.words;
        totals.fixes += e.fixes;
        totals.fillers += e.fillers;
        for t in tokens(&e.text) {
            vocabulary.insert(t.to_lowercase());
        }

        if let Some(ms) = e.ms.filter(|ms| *ms > 0) {
            timed_words += e.words;
            timed_ms += ms;
            totals.measured += 1;
        }
        if let Some(c) = e.confidence {
            conf_sum += c as f64;
            conf_n += 1;
        }
        if !e.app.is_empty() {
            let slot = per_app.entry(e.app.as_str()).or_insert((0, 0));
            slot.0 += e.words;
            slot.1 += 1;
        }

        if let Some(day) = local_day(e.at) {
            let slot = per_day.entry(day).or_insert((0, 0));
            slot.0 += e.words;
            slot.1 += 1;

            if Some(day) >= this_month {
                totals.words_this_month += e.words;
            } else if Some(day) >= prev_month {
                totals.words_last_month += e.words;
            }
        }
    }

    totals.apps = per_app.len();
    totals.vocabulary = vocabulary.len();
    totals.minutes_saved = (totals.words as f64 / TYPING_WPM).round() as usize;
    totals.books = totals.words as f64 / WORDS_PER_BOOK as f64;
    if timed_ms > 0 {
        totals.wpm = Some(timed_words as f64 / (timed_ms as f64 / 60_000.0));
    }
    if conf_n > 0 {
        totals.confidence = Some(conf_sum / conf_n as f64);
    }

    let mut days: Vec<(NaiveDate, usize, usize)> =
        per_day.into_iter().map(|(d, (w, c))| (d, w, c)).collect();
    days.sort_by_key(|(d, _, _)| *d);

    let mut apps: Vec<AppCount> = per_app
        .into_iter()
        .map(|(app, (words, dictations))| AppCount { app: app.to_string(), words, dictations })
        .collect();
    // Busiest first, then alphabetical so equal volumes hold a stable order.
    apps.sort_by(|a, b| b.words.cmp(&a.words).then_with(|| a.app.cmp(&b.app)));

    let active: Vec<NaiveDate> = days.iter().map(|(d, _, _)| *d).collect();

    Insights {
        totals,
        days: days
            .into_iter()
            .map(|(day, words, dictations)| DayCount {
                day: day.format("%Y-%m-%d").to_string(),
                words,
                dictations,
            })
            .collect(),
        apps,
        current_streak: current_streak(&active, today),
        longest_streak: longest_streak(&active),
    }
}

/// Consecutive dictated days ending today or yesterday.
///
/// Yesterday is allowed to end the run so a streak is not shown as broken before
/// the day is over: having not dictated yet today is not the same as having
/// given up.
fn current_streak(active: &[NaiveDate], today: NaiveDate) -> usize {
    let Some(last) = active.last() else { return 0 };
    let gap = (today - *last).num_days();
    if gap > 1 {
        return 0;
    }
    run_ending_at(active, active.len() - 1)
}

/// The longest run of consecutive days anywhere in the history.
fn longest_streak(active: &[NaiveDate]) -> usize {
    (0..active.len()).map(|i| run_ending_at(active, i)).max().unwrap_or(0)
}

/// Length of the consecutive-day run that ends at `index`.
fn run_ending_at(active: &[NaiveDate], index: usize) -> usize {
    let mut run = 1;
    while index >= run && active[index - run + 1] - active[index - run] == Duration::days(1) {
        run += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Local midnight for a day, so entries land on the day the tests expect.
    fn at_ms(d: NaiveDate) -> u64 {
        Local
            .from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
            .single()
            .expect("unambiguous local noon")
            .timestamp_millis() as u64
    }

    fn entry(text: &str, d: NaiveDate, words: usize, ms: Option<u64>) -> Entry {
        Entry {
            source: None,
            id: d.to_string(),
            text: text.to_string(),
            at: at_ms(d),
            app: "kitty".into(),
            words,
            ms,
            fixes: 0,
            fillers: 0,
            confidence: None,
        }
    }

    #[test]
    fn an_empty_history_reports_zeroes_not_panics() {
        let i = insights(&[], day(2026, 9, 17));
        assert_eq!(i.totals.words, 0);
        assert_eq!(i.totals.dictations, 0);
        assert_eq!(i.totals.apps, 0);
        assert_eq!(i.totals.vocabulary, 0);
        assert_eq!(i.totals.wpm, None, "no timed dictation means no rate to report");
        assert_eq!(i.totals.confidence, None);
        assert_eq!(i.current_streak, 0);
        assert_eq!(i.longest_streak, 0);
        assert_eq!(i.totals.books, 0.0);
        assert!(i.days.is_empty());
        assert!(i.apps.is_empty());
    }

    #[test]
    fn words_per_minute_ignores_untimed_dictations() {
        let e = vec![
            // 60 words in 30 s is 120 wpm, and only this one is measured.
            entry("w", day(2026, 9, 17), 60, Some(30_000)),
            // Untimed: including its words against someone else's seconds would
            // silently inflate the rate.
            entry("w", day(2026, 9, 17), 600, None),
            // A zero duration means unmeasured, so it is excluded too.
            entry("w", day(2026, 9, 17), 600, Some(0)),
        ];
        let t = insights(&e, day(2026, 9, 17)).totals;
        assert_eq!(t.measured, 1);
        assert!((t.wpm.unwrap() - 120.0).abs() < 1e-6);
    }

    #[test]
    fn words_per_minute_pools_words_rather_than_averaging_rates() {
        // Words are summed and divided by summed time, so a short fast
        // dictation cannot weigh as much as a long slow one. Here that gives
        // 60 words in 70 s = 51.43 wpm, where averaging the two rates (40 and
        // 120) would wrongly report 80.
        let e = vec![
            entry("w", day(2026, 9, 17), 40, Some(60_000)),
            entry("w", day(2026, 9, 17), 20, Some(10_000)),
        ];
        let t = insights(&e, day(2026, 9, 17)).totals;
        assert_eq!(t.measured, 2);
        assert!((t.wpm.unwrap() - 51.428_571).abs() < 0.001, "got {:?}", t.wpm);
    }

    #[test]
    fn vocabulary_counts_distinct_words_without_punctuation() {
        let e = vec![
            entry("Hello, world! hello again.", day(2026, 9, 17), 4, None),
            entry("well-known thing", day(2026, 9, 18), 2, None),
        ];
        // hello, world, again, well, known, thing — case folded, no punctuation.
        assert_eq!(insights(&e, day(2026, 9, 18)).totals.vocabulary, 6);
    }

    #[test]
    fn apps_are_ranked_by_volume() {
        let mut a = entry("w", day(2026, 9, 17), 10, None);
        a.app = "kitty".into();
        let mut b = entry("w", day(2026, 9, 17), 90, None);
        b.app = "google-chrome-beta".into();
        let mut c = entry("w", day(2026, 9, 17), 5, None);
        c.app = String::new(); // focus unknown — never a row of its own

        let i = insights(&[a, b, c], day(2026, 9, 17));
        assert_eq!(i.totals.apps, 2);
        assert_eq!(i.apps[0].app, "google-chrome-beta");
        assert_eq!(i.apps[0].words, 90);
        assert_eq!(i.apps[1].app, "kitty");
    }

    #[test]
    fn streak_runs_are_measured_on_consecutive_days() {
        // 1st, 2nd, 3rd, then a gap, then 10th and 11th.
        let mut e = Vec::new();
        for d in [1, 2, 3, 10, 11] {
            e.push(entry("w", day(2026, 9, d), 5, None));
        }
        let i = insights(&e, day(2026, 9, 11));
        assert_eq!(i.longest_streak, 3);
        assert_eq!(i.current_streak, 2, "the run ending on the last active day");
    }

    #[test]
    fn a_streak_survives_a_day_that_has_not_happened_yet() {
        let e = vec![entry("w", day(2026, 9, 16), 5, None)];
        // Today is the 17th and nothing has been dictated yet: the streak from
        // yesterday still stands rather than reading as broken.
        assert_eq!(insights(&e, day(2026, 9, 17)).current_streak, 1);
        // Two days of silence does break it.
        assert_eq!(insights(&e, day(2026, 9, 18)).current_streak, 0);
    }

    #[test]
    fn a_streak_starting_today_still_counts() {
        let e = vec![entry("w", day(2026, 9, 17), 5, None)];
        assert_eq!(insights(&e, day(2026, 9, 17)).current_streak, 1);
    }

    #[test]
    fn several_dictations_on_one_day_are_one_day_of_streak() {
        let e = vec![
            entry("w", day(2026, 9, 17), 5, None),
            entry("w", day(2026, 9, 17), 5, None),
            entry("w", day(2026, 9, 18), 5, None),
            entry("w", day(2026, 9, 18), 5, None),
        ];
        let i = insights(&e, day(2026, 9, 18));
        assert_eq!(i.longest_streak, 2);
        assert_eq!(i.current_streak, 2);
        assert_eq!(i.days.len(), 2, "one row per day, not per dictation");
        assert_eq!(i.days[0].dictations, 2);
        assert_eq!(i.days[0].words, 10);
    }

    #[test]
    fn months_are_split_on_local_calendar_boundaries() {
        let e = vec![
            entry("w", day(2026, 8, 31), 7, None),  // last month
            entry("w", day(2026, 9, 1), 11, None),  // this month, on the boundary
            entry("w", day(2026, 9, 17), 13, None), // this month
        ];
        let t = insights(&e, day(2026, 9, 17)).totals;
        assert_eq!(t.words_this_month, 24);
        assert_eq!(t.words_last_month, 7);
    }

    #[test]
    fn a_january_boundary_looks_back_to_december() {
        let e = vec![
            entry("w", day(2025, 12, 20), 9, None),
            entry("w", day(2026, 1, 2), 4, None),
        ];
        let t = insights(&e, day(2026, 1, 2)).totals;
        assert_eq!(t.words_this_month, 4);
        assert_eq!(t.words_last_month, 9);
    }

    #[test]
    fn derived_figures_use_their_stated_constants() {
        let e = vec![entry("w", day(2026, 9, 17), 200_000, None)];
        let t = insights(&e, day(2026, 9, 17)).totals;
        assert_eq!(t.minutes_saved, 5_000); // 200k words at 40 wpm
        assert!((t.books - 2.0).abs() < 1e-9); // 200k words at 100k per book
    }

    #[test]
    fn fixes_fillers_and_confidence_are_summed() {
        let mut a = entry("w", day(2026, 9, 17), 5, None);
        a.fixes = 2;
        a.fillers = 3;
        a.confidence = Some(0.9);
        let mut b = entry("w", day(2026, 9, 18), 5, None);
        b.fixes = 1;
        b.confidence = Some(0.7);

        let t = insights(&[a, b], day(2026, 9, 18)).totals;
        assert_eq!(t.fixes, 3);
        assert_eq!(t.fillers, 3);
        // Mean over the entries that reported one; b without fillers still
        // counts for confidence, and its absence of fillers adds nothing.
        // The tolerance is 1e-6 rather than 1e-9 because confidence arrives as
        // an f32, so (0.9 + 0.7) / 2 cannot land exactly on 0.8 once widened.
        assert!((t.confidence.unwrap() - 0.8).abs() < 1e-6, "got {:?}", t.confidence);
    }

    #[test]
    fn days_come_back_in_order() {
        let e = vec![
            entry("w", day(2026, 9, 18), 5, None),
            entry("w", day(2026, 9, 16), 5, None),
            entry("w", day(2026, 9, 17), 5, None),
        ];
        let i = insights(&e, day(2026, 9, 18));
        let days: Vec<&str> = i.days.iter().map(|d| d.day.as_str()).collect();
        assert_eq!(days, vec!["2026-09-16", "2026-09-17", "2026-09-18"]);
    }
}
