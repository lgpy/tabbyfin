//! Chapters and skippable segments (intro, recap, credits…) of the playing item.
//!
//! Segments come from the server's segment provider plugins. Without any, they're guessed from
//! chapter names, which many releases set ("Intro", "Opening", "OP", "Credits", "ED"…).

use std::time::Duration;

use jellyfin::models::{ChapterInfo, MediaSegmentDto, MediaSegmentType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub start: Duration,
    pub name: Option<String>,
    /// The chapter's image, fetched by its position in the server's list (`index`).
    pub image_tag: Option<String>,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Intro,
    Recap,
    Preview,
    Commercial,
    /// Credits: where the next episode is offered.
    Outro,
}

impl SegmentKind {
    /// The skip button's label.
    pub fn skip_label(self) -> &'static str {
        match self {
            SegmentKind::Intro => "Skip intro",
            SegmentKind::Recap => "Skip recap",
            SegmentKind::Preview => "Skip preview",
            SegmentKind::Commercial => "Skip ad",
            SegmentKind::Outro => "Skip credits",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start: Duration,
    pub end: Duration,
}

pub fn chapters(chapters: &[ChapterInfo]) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = chapters
        .iter()
        .enumerate()
        .map(|(index, c)| Chapter {
            start: c.start_position_ticks.into(),
            name: c.name.clone().filter(|n| !n.trim().is_empty()),
            image_tag: c.image_tag.clone(),
            index,
        })
        .collect();
    chapters.sort_by_key(|c| c.start);
    chapters
}

pub fn from_server(segments: &[MediaSegmentDto]) -> Vec<Segment> {
    segments
        .iter()
        .filter_map(|s| {
            let kind = match s.kind {
                MediaSegmentType::Intro => SegmentKind::Intro,
                MediaSegmentType::Recap => SegmentKind::Recap,
                MediaSegmentType::Preview => SegmentKind::Preview,
                MediaSegmentType::Commercial => SegmentKind::Commercial,
                MediaSegmentType::Outro => SegmentKind::Outro,
                MediaSegmentType::Unknown | MediaSegmentType::Other => return None,
            };
            let (start, end): (Duration, Duration) = (s.start_ticks.into(), s.end_ticks.into());
            (end > start).then_some(Segment { kind, start, end })
        })
        .collect()
}

/// Segments guessed from chapter names. Each runs to the next chapter, or to `runtime` for
/// the last one.
pub fn from_chapters(chapters: &[Chapter], runtime: Option<Duration>) -> Vec<Segment> {
    chapters
        .iter()
        .enumerate()
        .filter_map(|(i, chapter)| {
            let kind = kind_for_name(chapter.name.as_deref()?)?;
            let end = chapters.get(i + 1).map(|next| next.start).or(runtime)?;
            (end > chapter.start).then_some(Segment {
                kind,
                start: chapter.start,
                end,
            })
        })
        .collect()
}

fn kind_for_name(name: &str) -> Option<SegmentKind> {
    let name = name.trim().to_lowercase();
    // The first word, so "Opening Credits" and "OP - Song Title" count but "Operation" doesn't.
    let first = name.split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default();
    match first {
        "intro" | "opening" | "op" => Some(SegmentKind::Intro),
        "recap" | "previously" => Some(SegmentKind::Recap),
        "preview" => Some(SegmentKind::Preview),
        "next" if name.starts_with("next episode") => Some(SegmentKind::Preview),
        "credits" | "ending" | "ed" | "outro" => Some(SegmentKind::Outro),
        "end" if name.starts_with("end credits") => Some(SegmentKind::Outro),
        _ => None,
    }
}

/// The segment to offer skipping at `position`: not credits (the up-next card handles those),
/// and not in its last second.
pub fn skippable(segments: &[Segment], position: Duration) -> Option<Segment> {
    segments
        .iter()
        .find(|s| s.kind != SegmentKind::Outro && s.start <= position && position + Duration::from_secs(1) < s.end)
        .copied()
}

/// PgDn: the next chapter's start. `None` in the last chapter (rather than ending the file).
pub fn next_chapter(chapters: &[Chapter], position: Duration) -> Option<Duration> {
    chapters
        .iter()
        .map(|c| c.start)
        .find(|&start| start > position + Duration::from_secs(1))
}

/// PgUp: the current chapter's start, or the previous chapter's when just past a start.
pub fn prev_chapter(chapters: &[Chapter], position: Duration) -> Option<Duration> {
    let starts: Vec<Duration> = chapters.iter().map(|c| c.start).filter(|&s| s <= position).collect();
    match starts.as_slice() {
        [] => None,
        [.., previous, current] if position < *current + Duration::from_secs(3) => Some(*previous),
        [.., current] => Some(*current),
    }
}

/// The chapter playing at `position`.
pub fn current_chapter(chapters: &[Chapter], position: Duration) -> Option<&Chapter> {
    chapters.iter().rev().find(|c| c.start <= position)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Chapter, Segment, SegmentKind, current_chapter, from_chapters, kind_for_name, next_chapter, prev_chapter,
        skippable,
    };

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn chapter(start: u64, name: &str) -> Chapter {
        Chapter {
            start: secs(start),
            name: Some(name.into()),
            image_tag: None,
            index: 0,
        }
    }

    #[test]
    fn names() {
        for (name, kind) in [
            ("Intro", Some(SegmentKind::Intro)),
            ("Opening Credits", Some(SegmentKind::Intro)),
            ("OP - Some Song", Some(SegmentKind::Intro)),
            ("Operation", None),
            ("Recap", Some(SegmentKind::Recap)),
            ("Previously on", Some(SegmentKind::Recap)),
            ("Preview", Some(SegmentKind::Preview)),
            ("Next Episode", Some(SegmentKind::Preview)),
            ("Next", None),
            ("Credits", Some(SegmentKind::Outro)),
            ("ED", Some(SegmentKind::Outro)),
            ("Ending", Some(SegmentKind::Outro)),
            ("End Credits", Some(SegmentKind::Outro)),
            ("The End", None),
            ("End", None),
            ("Chapter 1", None),
        ] {
            assert_eq!(kind_for_name(name), kind, "{name}");
        }
    }

    #[test]
    fn segments_run_to_the_next_chapter() {
        let chapters = [chapter(0, "Intro"), chapter(10, "Episode"), chapter(45, "Credits")];
        assert_eq!(
            from_chapters(&chapters, Some(secs(60))),
            [
                Segment {
                    kind: SegmentKind::Intro,
                    start: secs(0),
                    end: secs(10)
                },
                Segment {
                    kind: SegmentKind::Outro,
                    start: secs(45),
                    end: secs(60)
                },
            ]
        );
        // Without a runtime, the last chapter's end is unknown.
        assert_eq!(from_chapters(&chapters, None).len(), 1);
    }

    #[test]
    fn skip_offered_inside_intro_only() {
        let segments = from_chapters(
            &[chapter(0, "Intro"), chapter(10, "Episode"), chapter(45, "Credits")],
            Some(secs(60)),
        );
        assert_eq!(skippable(&segments, secs(3)).map(|s| s.end), Some(secs(10)));
        assert_eq!(skippable(&segments, Duration::from_millis(9500)), None);
        assert_eq!(skippable(&segments, secs(20)), None);
        assert_eq!(skippable(&segments, secs(50)), None);
    }

    #[test]
    fn chapter_keys() {
        let chapters = [chapter(0, "Intro"), chapter(10, "Episode"), chapter(45, "Credits")];
        assert_eq!(next_chapter(&chapters, secs(3)), Some(secs(10)));
        assert_eq!(next_chapter(&chapters, Duration::from_millis(9500)), Some(secs(45)));
        assert_eq!(next_chapter(&chapters, secs(50)), None);
        assert_eq!(prev_chapter(&chapters, secs(30)), Some(secs(10)));
        assert_eq!(prev_chapter(&chapters, secs(11)), Some(secs(0)));
        assert_eq!(prev_chapter(&chapters, secs(1)), Some(secs(0)));
        assert_eq!(prev_chapter(&[], secs(1)), None);
    }

    #[test]
    fn current_chapter_at_position() {
        let chapters = [chapter(0, "Intro"), chapter(10, "Episode")];
        assert_eq!(
            current_chapter(&chapters, secs(12)).and_then(|c| c.name.as_deref()),
            Some("Episode")
        );
        assert_eq!(
            current_chapter(&chapters, secs(0)).and_then(|c| c.name.as_deref()),
            Some("Intro")
        );
    }
}
