//! Russian names for heroes and catalog sections.
//!
//! Hero names come from the game's own localization files, so they match what
//! players see in Deadlock and new heroes need no launcher update. Section
//! names are a short fixed translation of GameBanana's root categories.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::steam::GamePaths;

/// English GameBanana section name → Russian name, in sidebar order.
const SECTIONS: [(&str, &str); 8] = [
    ("HUD", "Интерфейс"),
    ("Quality of Life/Fixes", "Удобство и фиксы"),
    ("Gameplay Modifications", "Геймплей"),
    ("Model Replacement", "Модели и объекты"),
    ("Animations", "Анимации"),
    ("Maps", "Карты"),
    ("Other/Misc", "Разное"),
    ("Skins", "Скины"),
];

/// Translated section name, or the original when it is unknown.
pub fn section(english: &str) -> &str {
    SECTIONS
        .iter()
        .find(|(en, _)| en.eq_ignore_ascii_case(english))
        .map_or(english, |(_, ru)| ru)
}

/// Sidebar position of a section; unknown sections go last.
pub fn section_order(english: &str) -> usize {
    SECTIONS
        .iter()
        .position(|(en, _)| en.eq_ignore_ascii_case(english))
        .unwrap_or(SECTIONS.len())
}

#[derive(Default)]
pub struct HeroNames {
    by_english: HashMap<String, String>,
}

impl HeroNames {
    pub fn load(paths: &GamePaths) -> Self {
        let dir = paths
            .citadel()
            .join("resource")
            .join("localization")
            .join("citadel_gc_hero_names");
        let read = |lang: &str| read_tokens(&dir.join(format!("citadel_gc_hero_names_{lang}.txt")));
        Self::from_tokens(&read("english"), &read("russian"))
    }

    fn from_tokens(english: &HashMap<String, String>, russian: &HashMap<String, String>) -> Self {
        let by_english = english
            .iter()
            .filter_map(|(token, en)| Some((en.to_lowercase(), russian.get(token)?.clone())))
            .collect();
        Self { by_english }
    }

    /// Russian hero name, or the English one when the game has no translation.
    pub fn get<'a>(&'a self, english: &'a str) -> &'a str {
        self.by_english
            .get(&english.to_lowercase())
            .map_or(english, String::as_str)
    }
}

fn read_tokens(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .map(|text| parse_tokens(&text))
        .unwrap_or_default()
}

/// Parses `"hero_haze:n"  "#|f|#Пелена"` lines into `hero_haze → Пелена`.
/// Search aliases (`hero_haze_search:n`) are skipped.
fn parse_tokens(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            let token = quoted.next()?.strip_suffix(":n")?;
            let value = quoted.next()?;
            (token.starts_with("hero_") && !token.ends_with("_search"))
                .then(|| (token.to_string(), strip_gender(value).to_string()))
        })
        .collect()
}

/// Drops the `#|m|#` grammatical gender marker the game puts before names.
fn strip_gender(value: &str) -> &str {
    match value.strip_prefix("#|") {
        Some(rest) => rest.split_once("|#").map_or(value, |(_, name)| name),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = "\"lang\"\n{\n\t\"Tokens\"\n\t{\n\t\t\"hero_haze:n\" \"Haze\"\n\t\t\"hero_haze_search:n\" \"Haze\"\n\t\t\"hero_krill:n\" \"Mo & Krill\"\n\t}\n}";
    const RU: &str =
        "\t\t\"hero_haze:n\" \"#|f|#Пелена\"\n\t\t\"hero_krill:n\" \"#|m|#Мо и Мелкий\"\n";

    #[test]
    fn maps_english_hero_names_to_russian() {
        let names = HeroNames::from_tokens(&parse_tokens(EN), &parse_tokens(RU));
        assert_eq!(names.get("Haze"), "Пелена");
        assert_eq!(names.get("Mo & Krill"), "Мо и Мелкий");
    }

    #[test]
    fn unknown_hero_keeps_english_name() {
        let names = HeroNames::from_tokens(&parse_tokens(EN), &parse_tokens(RU));
        assert_eq!(names.get("Brand New Hero"), "Brand New Hero");
    }

    #[test]
    fn skips_search_aliases() {
        assert_eq!(parse_tokens(EN).len(), 2);
    }

    #[test]
    fn translates_sections_and_orders_unknown_last() {
        assert_eq!(section("HUD"), "Интерфейс");
        assert_eq!(section("Something New"), "Something New");
        assert!(section_order("HUD") < section_order("Maps"));
        assert_eq!(section_order("Something New"), SECTIONS.len());
    }
}
