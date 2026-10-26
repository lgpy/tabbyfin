//! A library's or person's items as a virtualised grid, with sort, filters and the letter bar:
//! the toolbar's sort, filter and resolution choices, the years it can filter by, and where the
//! jump bar's letters scroll to. The grid and its toolbar arrive with U5.

use jellyfin::models::{ItemFilter, ItemSortBy};

/// Sort choices: config key, label, server sort, and whether it starts descending.
pub const SORTS: [(&str, &str, ItemSortBy, bool); 6] = [
    ("name", "Name", ItemSortBy::SortName, false),
    ("added", "Date added", ItemSortBy::DateCreated, true),
    ("released", "Release date", ItemSortBy::PremiereDate, true),
    ("rating", "Rating", ItemSortBy::CommunityRating, true),
    ("runtime", "Runtime", ItemSortBy::Runtime, false),
    ("random", "Random", ItemSortBy::Random, false),
];

/// Filter choices: config key, label, server filter.
pub const FILTERS: [(&str, &str, Option<ItemFilter>); 5] = [
    ("all", "All", None),
    ("unwatched", "Unwatched", Some(ItemFilter::IsUnplayed)),
    ("watched", "Watched", Some(ItemFilter::IsPlayed)),
    ("favorites", "Favorites", Some(ItemFilter::IsFavorite)),
    ("in-progress", "In progress", Some(ItemFilter::IsResumable)),
];

/// Resolution choices: config key, label, and video width bounds (4K from 3800 wide, HD from
/// 1200, which 720p's 1280 clears). 12.2 goes by an item's widest version, 10.11 by its first.
pub const RESOLUTIONS: [(&str, &str, Option<u32>, Option<u32>); 4] = [
    ("any", "Any resolution", None, None),
    ("4k", "4K", Some(3800), None),
    ("hd", "HD", Some(1200), Some(3799)),
    ("sd", "SD", None, Some(1199)),
];

/// The value picked from an "any, then `values`" menu; `None` for "any".
pub fn chosen(values: &[String], index: i32) -> Option<String> {
    usize::try_from(index - 1).ok().and_then(|i| values.get(i)).cloned()
}

/// Year menu keys, newest first: each year, preceded by its decade ("2020s") when the library
/// has more than one year from it.
pub fn year_choices(years: &[i32]) -> Vec<String> {
    let mut years = years.to_vec();
    years.sort_unstable_by(|a, b| b.cmp(a));
    years.dedup();
    let decade = |year: i32| year - year.rem_euclid(10);
    let mut keys = Vec::new();
    for (i, &year) in years.iter().enumerate() {
        let starts_decade = i == 0 || decade(years[i - 1]) != decade(year);
        if starts_decade && years.iter().filter(|&&y| decade(y) == decade(year)).count() > 1 {
            keys.push(format!("{}s", decade(year)));
        }
        keys.push(year.to_string());
    }
    keys
}

/// The years a year menu key covers.
pub fn years_of(key: &str) -> Vec<i32> {
    match key.strip_suffix('s').and_then(|decade| decade.parse::<i32>().ok()) {
        Some(decade) => (decade..decade + 10).collect(),
        None => key.parse().ok().into_iter().collect(),
    }
}

/// Where a letter of the jump bar scrolls to.
#[derive(Debug, PartialEq)]
pub enum Jump {
    Top,
    Bottom,
    /// Past the names that sort before this letter (counted by the server).
    After(char),
}

/// `#` is the first row ascending and the last descending. A letter starts where the names
/// before it end; descending, that's after every name from the next letter on.
pub fn jump_target(letter: char, descending: bool) -> Jump {
    match (letter, descending) {
        ('#', false) | ('Z', true) => Jump::Top,
        ('#', true) => Jump::Bottom,
        (letter, false) => Jump::After(letter),
        (letter, true) => Jump::After(char::from(letter as u8 + 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Jump, chosen, jump_target, year_choices, years_of};

    #[test]
    fn letters_jump_past_the_names_before_them() {
        assert_eq!(jump_target('#', false), Jump::Top);
        assert_eq!(jump_target('M', false), Jump::After('M'));
        assert_eq!(jump_target('#', true), Jump::Bottom);
        assert_eq!(jump_target('M', true), Jump::After('N'));
        assert_eq!(jump_target('Z', true), Jump::Top);
    }

    #[test]
    fn years_are_grouped_under_decades_with_more_than_one() {
        assert_eq!(
            year_choices(&[2024, 1999, 2025, 2024]),
            ["2020s", "2025", "2024", "1999"]
        );
        assert_eq!(year_choices(&[]), Vec::<String>::new());
        assert_eq!(years_of("2020s"), (2020..2030).collect::<Vec<_>>());
        assert_eq!(years_of("1999"), [1999]);
        assert_eq!(years_of("nonsense"), Vec::<i32>::new());
    }

    #[test]
    fn the_first_choice_is_any() {
        let values = ["Comedy".to_owned(), "Drama".to_owned()];
        assert_eq!(chosen(&values, 0), None);
        assert_eq!(chosen(&values, 2).as_deref(), Some("Drama"));
        assert_eq!(chosen(&values, 3), None);
    }
}
