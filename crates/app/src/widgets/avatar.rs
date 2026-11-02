//! A person as a pastel circle with their initial, for where there's no picture: a saved
//! account on the sign-in screen, and anywhere else a user is listed. A cast card without a
//! photo takes the same pastel ([`colour`]).

use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Align2, Color32, FontId, Pos2, Ui};

use crate::theme;

/// Paints `name`'s avatar as a circle of `radius` around `center`. The colour comes from the
/// name, so a person keeps theirs from one launch to the next and two people rarely share one.
pub fn paint(ui: &Ui, center: Pos2, radius: f32, name: &str) {
    let tokens = theme::tokens_of(ui);
    let painter = ui.painter();
    painter.circle_filled(center, radius, colour(tokens, name));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        initial(name),
        FontId::proportional(radius * 0.95),
        tokens.avatar_foreground,
    );
}

/// `name`'s pastel: their avatar's colour, and the fill of a person card without a picture.
pub fn colour(tokens: &theme::Tokens, name: &str) -> Color32 {
    tokens.avatars[colour_index(name, tokens.avatars.len())]
}

/// The first letter of `name`, upper-cased, or "?" for an empty one.
fn initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
}

/// Which of `count` colours `name` gets. `DefaultHasher::new` uses fixed keys, so this is the
/// same in every run.
fn colour_index(name: &str, count: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    (hasher.finish() % count as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::{colour_index, initial};

    #[test]
    fn the_initial_is_the_first_letter_upper_cased() {
        assert_eq!(initial("dev"), "D");
        assert_eq!(initial("  émile"), "É");
        assert_eq!(initial(""), "?");
    }

    #[test]
    fn a_name_keeps_its_colour() {
        assert_eq!(colour_index("dev", 5), colour_index("dev", 5));
        assert!(colour_index("alex", 5) < 5);
    }
}
