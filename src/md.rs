#![allow(unused_variables)]
#![allow(unused_imports)]

use crate::consts::{COMMON_SAFE_ASCII_CHARACTERS, UTF8_MAXIMAL_ALLOCATION};
use crate::utils::{is_suspiciously_successive_range, remove_accent, unicode_range};
use bitflags::{bitflags, Flags};
use cached::proc_macro::cached;
use cached::UnboundCache;
use log::trace;
use ordered_float::OrderedFloat;
use unic::char::property::EnumeratedCharProperty;
use unic::ucd::{is_white_space, GeneralCategory, Name};

pub(crate) mod structs;

use structs::{MessDetectorChar, MessDetectorCharFlags, MessDetectorPlugin};


//
// Mess detection module
//

//
// TooManySymbolOrPunctuationPlugin implementation
//
#[derive(Default)]
struct TooManySymbolOrPunctuationPlugin {
    punctuation_count: u64,
    symbol_count: u64,
    character_count: u64,
    last_printable_char: Option<MessDetectorChar>,
}

impl MessDetectorPlugin for TooManySymbolOrPunctuationPlugin {
    fn eligible(&self, character: &MessDetectorChar) -> bool {
        !character.is(MessDetectorCharFlags::UNPRINTABLE)
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        self.character_count += 1;
        if (self.last_printable_char.is_none() || *character != self.last_printable_char.unwrap())
            && !character.is(MessDetectorCharFlags::COMMON_SAFE)
        {
            if character.is(MessDetectorCharFlags::PUNCTUATION) {
                self.punctuation_count += 1;
            } else if !character.is(MessDetectorCharFlags::NUMERIC)
                && character.is(MessDetectorCharFlags::SYMBOL)
                && !character.is(MessDetectorCharFlags::EMOTICON)
            {
                self.symbol_count += 2;
            }
        }
        self.last_printable_char = Some(*character);
    }
    fn ratio(&self) -> f32 {
        if self.character_count == 0 {
            return 0.0;
        }
        let ratio_of_punctuation =
            (self.punctuation_count + self.symbol_count) as f32 / (self.character_count as f32);
        if ratio_of_punctuation >= 0.3 {
            ratio_of_punctuation
        } else {
            0.0
        }
    }
}

//
// TooManyAccentuatedPlugin implementation
//

#[derive(Default)]
struct TooManyAccentuatedPlugin {
    character_count: u64,
    accentuated_count: u64,
}

impl MessDetectorPlugin for TooManyAccentuatedPlugin {
    fn eligible(&self, character: &MessDetectorChar) -> bool {
        character.is(MessDetectorCharFlags::ALPHABETIC)
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        self.character_count += 1;
        if character.is(MessDetectorCharFlags::ACCENTUATED) {
            self.accentuated_count += 1;
        }
    }
    fn ratio(&self) -> f32 {
        (self.character_count >= 8)
            .then_some(self.accentuated_count as f32 / self.character_count as f32)
            .filter(|&ratio| ratio >= 0.35)
            .unwrap_or(0.0)
    }
}

//
// UnprintablePlugin implementation
//

#[derive(Default)]
struct UnprintablePlugin {
    character_count: u64,
    unprintable_count: u64,
}

impl MessDetectorPlugin for UnprintablePlugin {
    fn eligible(&self, character: &MessDetectorChar) -> bool {
        true
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        if character.is(MessDetectorCharFlags::UNPRINTABLE) {
            self.unprintable_count += 1;
        }
        self.character_count += 1;
    }
    fn ratio(&self) -> f32 {
        if self.character_count == 0 {
            return 0.0;
        }
        (self.unprintable_count as f32 * 8.0) / self.character_count as f32
    }
}

//
// SuspiciousDuplicateAccentPlugin implementation
//
#[derive(Default)]
struct SuspiciousDuplicateAccentPlugin {
    character_count: u64,
    successive_count: u64,
    last_latin_character: Option<MessDetectorChar>,
}

impl MessDetectorPlugin for SuspiciousDuplicateAccentPlugin {
    fn eligible(&self, character: &MessDetectorChar) -> bool {
        character.is(MessDetectorCharFlags::ALPHABETIC)
            && character.is(MessDetectorCharFlags::LATIN)
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        self.character_count += 1;
        if self.last_latin_character.is_some()
            && character.is(MessDetectorCharFlags::ACCENTUATED)
            && self
                .last_latin_character
                .unwrap()
                .is(MessDetectorCharFlags::ACCENTUATED)
        {
            if character.is(MessDetectorCharFlags::UPPERCASE)
                && self
                    .last_latin_character
                    .unwrap()
                    .is(MessDetectorCharFlags::UPPERCASE)
            {
                self.successive_count += 1;
            }

            // Worse if its the same char duplicated with different accent.
            if remove_accent(character.character)
                == remove_accent(self.last_latin_character.unwrap().character)
            {
                self.successive_count += 1;
            }
        }
        self.last_latin_character = Some(*character);
    }
    fn ratio(&self) -> f32 {
        if self.character_count == 0 {
            return 0.0;
        }
        (self.successive_count as f32 * 2.0) / self.character_count as f32
    }
}

//
// SuspiciousRangePlugin implementation
//
#[derive(Default)]
struct SuspiciousRangePlugin {
    character_count: u64,
    suspicious_successive_range_count: u64,
    last_printable_char: Option<MessDetectorChar>,
}

impl MessDetectorPlugin for SuspiciousRangePlugin {
    fn eligible(&self, character: &MessDetectorChar) -> bool {
        !character.is(MessDetectorCharFlags::UNPRINTABLE)
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        self.character_count += 1;

        if character.is(MessDetectorCharFlags::WHITESPACE)
            || character.is(MessDetectorCharFlags::PUNCTUATION)
            || character.is(MessDetectorCharFlags::COMMON_SAFE)
        {
            self.last_printable_char = None;
            return;
        }

        if self.last_printable_char.is_none() {
            self.last_printable_char = Some(*character);
            return;
        }

        if is_suspiciously_successive_range(
            self.last_printable_char.unwrap().unicode_range,
            character.unicode_range,
        ) {
            self.suspicious_successive_range_count += 1;
        }

        self.last_printable_char = Some(*character);
    }
    fn ratio(&self) -> f32 {
        (self.character_count > 0)
            .then_some(
                ((self.suspicious_successive_range_count as f32) * 2.0)
                    / self.character_count as f32,
            )
            .filter(|&ratio| ratio >= 0.1)
            .unwrap_or(0.0)
    }
}

//
// SuperWeirdWordPlugin implementation
//

#[derive(Default)]
struct SuperWeirdWordPlugin {
    character_count: u64,
    word_count: u64,
    bad_word_count: u64,
    foreign_long_count: u64,
    is_current_word_bad: bool,
    foreign_long_watch: bool,
    bad_character_count: u64,
    buffer_accent_count: u64,
    buffer: Vec<MessDetectorChar>,
}

impl MessDetectorPlugin for SuperWeirdWordPlugin {
    fn eligible(&self, character: &MessDetectorChar) -> bool {
        true
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        if character.is(MessDetectorCharFlags::ASCII_ALPHABETIC) {
            self.buffer.push(*character);
            if character.is(MessDetectorCharFlags::ACCENTUATED) {
                self.buffer_accent_count += 1;
            }
            self.foreign_long_watch |= (!character.is(MessDetectorCharFlags::LATIN)
                || character.is(MessDetectorCharFlags::ACCENTUATED))
                && !character.is(MessDetectorCharFlags::CJK)
                && !character.is(MessDetectorCharFlags::HANGUL)
                && !character.is(MessDetectorCharFlags::KATAKANA)
                && !character.is(MessDetectorCharFlags::HIRAGANA)
                && !character.is(MessDetectorCharFlags::THAI);
            return;
        }
        if self.buffer.is_empty() {
            return;
        }

        if character.is(MessDetectorCharFlags::WHITESPACE)
            || character.is(MessDetectorCharFlags::PUNCTUATION)
            || character.is(MessDetectorCharFlags::SEPARATOR)
        {
            self.word_count += 1;
            let buffer_length = self.buffer.len();
            self.character_count += buffer_length as u64;

            if buffer_length >= 4 {
                if (self.buffer_accent_count as f32 / buffer_length as f32) > 0.34 {
                    self.is_current_word_bad = true;
                }

                // Word/Buffer ending with an upper case accentuated letter are so rare,
                // that we will consider them all as suspicious. Same weight as foreign_long suspicious.
                let last_char = self.buffer.last().unwrap();
                if last_char.is(MessDetectorCharFlags::ACCENTUATED)
                    && last_char.is(MessDetectorCharFlags::UPPERCASE)
                {
                    self.foreign_long_count += 1;
                    self.is_current_word_bad = true;
                }
            }
            if buffer_length >= 24 && self.foreign_long_watch {
                let uppercase_count = self
                    .buffer
                    .iter()
                    .filter(|&c| c.is(MessDetectorCharFlags::UPPERCASE))
                    .count();
                let mut probable_camel_cased: bool = false;

                if uppercase_count > 0 && (uppercase_count as f32 / buffer_length as f32) <= 0.3 {
                    probable_camel_cased = true;
                }

                if !probable_camel_cased {
                    self.foreign_long_count += 1;
                    self.is_current_word_bad = true;
                }
            }

            if self.is_current_word_bad {
                self.bad_word_count += 1;
                self.bad_character_count += self.buffer.len() as u64;
                self.is_current_word_bad = false;
            }

            self.foreign_long_watch = false;
            self.buffer.clear();
            self.buffer_accent_count = 0;
        } else if !character.is(MessDetectorCharFlags::WEIRD_SAFE)
            && !character.is(MessDetectorCharFlags::ASCII_DIGIT)
            && character.is(MessDetectorCharFlags::SYMBOL)
        {
            self.is_current_word_bad = true;
            self.buffer.push(*character);
        }
    }
    fn ratio(&self) -> f32 {
        if self.word_count <= 10 && self.foreign_long_count == 0 {
            return 0.0;
        }
        self.bad_character_count as f32 / self.character_count as f32
    }
}

//
// CjkInvalidStopPlugin implementation
//
// GB(Chinese) based encoding often render the stop incorrectly when the content does not fit and
// can be easily detected. Searching for the overuse of '丅' and '丄'.
#[derive(Default)]
struct CjkInvalidStopPlugin {
    wrong_stop_count: u64,
    cjk_character_count: u64,
}

impl MessDetectorPlugin for CjkInvalidStopPlugin {
    fn eligible(&self, _: &MessDetectorChar) -> bool {
        true
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        if "丅丄".contains(character.character) {
            self.wrong_stop_count += 1;
            return;
        }
        if character.is(MessDetectorCharFlags::CJK) {
            self.cjk_character_count += 1;
        }
    }
    fn ratio(&self) -> f32 {
        if self.cjk_character_count < 16 {
            return 0.0;
        }
        self.wrong_stop_count as f32 / self.cjk_character_count as f32
    }
}

//
// ArchaicUpperLowerPlugin implementation
//

struct ArchaicUpperLowerPlugin {
    buf: bool,
    current_ascii_only: bool,
    character_count_since_last_sep: u64,
    successive_upper_lower_count: u64,
    successive_upper_lower_count_final: u64,
    character_count: u64,
    last_alpha_seen: Option<MessDetectorChar>,
}

impl Default for ArchaicUpperLowerPlugin {
    fn default() -> Self {
        ArchaicUpperLowerPlugin {
            buf: false,
            current_ascii_only: true,
            character_count_since_last_sep: 0,
            successive_upper_lower_count: 0,
            successive_upper_lower_count_final: 0,
            character_count: 0,
            last_alpha_seen: None,
        }
    }
}

impl MessDetectorPlugin for ArchaicUpperLowerPlugin {
    fn eligible(&self, _: &MessDetectorChar) -> bool {
        true
    }
    fn feed(&mut self, character: &MessDetectorChar) {
        if !(character.is(MessDetectorCharFlags::ALPHABETIC)
            && character.is(MessDetectorCharFlags::CASE_VARIABLE))
            && self.character_count_since_last_sep > 0
        {
            if self.character_count_since_last_sep <= 64
                && !character.is(MessDetectorCharFlags::ASCII_DIGIT)
                && !self.current_ascii_only
            {
                self.successive_upper_lower_count_final += self.successive_upper_lower_count;
            }

            self.successive_upper_lower_count = 0;
            self.character_count_since_last_sep = 0;
            self.last_alpha_seen = None;
            self.buf = false;
            self.character_count += 1;
            self.current_ascii_only = true;

            return;
        }

        self.current_ascii_only &= character.is(MessDetectorCharFlags::ASCII);

        if let Some(tmp_last_alpha) = self.last_alpha_seen {
            if (character.is(MessDetectorCharFlags::UPPERCASE)
                && tmp_last_alpha.is(MessDetectorCharFlags::LOWERCASE))
                || (character.is(MessDetectorCharFlags::LOWERCASE)
                    && tmp_last_alpha.is(MessDetectorCharFlags::UPPERCASE))
            {
                if self.buf {
                    self.successive_upper_lower_count += 2;
                    self.buf = false;
                } else {
                    self.buf = true;
                }
            } else {
                self.buf = false;
            }
        }

        self.character_count += 1;
        self.character_count_since_last_sep += 1;
        self.last_alpha_seen = Some(*character);
    }
    fn ratio(&self) -> f32 {
        if self.character_count == 0 {
            return 0.0;
        }
        self.successive_upper_lower_count_final as f32 / self.character_count as f32
    }
}

// Compute a mess ratio given a decoded bytes sequence. The maximum threshold does stop the computation earlier.
#[cached(size = 2048)]
pub(crate) fn mess_ratio(
    decoded_sequence: String,
    maximum_threshold: Option<OrderedFloat<f32>>,
) -> f32 {
    let maximum_threshold = f32::from(maximum_threshold.unwrap_or(OrderedFloat(0.2)));
    let mut detectors: Vec<Box<dyn MessDetectorPlugin>> = vec![
        Box::<TooManySymbolOrPunctuationPlugin>::default(),
        Box::<TooManyAccentuatedPlugin>::default(),
        Box::<UnprintablePlugin>::default(),
        Box::<SuspiciousRangePlugin>::default(),
        Box::<SuspiciousDuplicateAccentPlugin>::default(),
        Box::<SuperWeirdWordPlugin>::default(),
        Box::<CjkInvalidStopPlugin>::default(),
        Box::<ArchaicUpperLowerPlugin>::default(),
    ];

    let length = decoded_sequence.chars().count();
    let mut mean_mess_ratio: f32 = 0.0;
    let intermediary_mean_mess_ratio_calc: usize = match length {
        0..=510 => 32,
        511..=1023 => 64,
        _ => 128,
    };
    // Traverse through chars and detectors
    for (index, ch) in decoded_sequence
        .chars()
        .chain(std::iter::once('\n'))
        .enumerate()
    {
        let mess_char = MessDetectorChar::new(ch);
        detectors
            .iter_mut()
            .filter(|detector| detector.eligible(&mess_char))
            .for_each(|detector| detector.feed(&mess_char));

        if (index > 0 && index.rem_euclid(intermediary_mean_mess_ratio_calc) == 0)
            || index == length
        {
            mean_mess_ratio = detectors.iter().map(|x| x.ratio()).sum();
            if mean_mess_ratio >= maximum_threshold {
                break;
            }
        }
    }

    trace!(
        "Mess-detector extended-analysis start: \
        intermediary_mean_mess_ratio_calc={}, \
        mean_mess_ratio={}, \
        maximum_threshold={}",
        intermediary_mean_mess_ratio_calc,
        mean_mess_ratio,
        maximum_threshold,
    );

    /*if decoded_sequence.len() > 16 {
        trace!(
            "Chunk: {} ..... {}",
            &decoded_sequence[..decoded_sequence
                .char_indices()
                .nth(16)
                .map(|(i, _)| i)
                .unwrap_or(decoded_sequence.chars().count())],
            &decoded_sequence[decoded_sequence
                .char_indices()
                .nth(decoded_sequence.chars().count() - 16)
                .map(|(i, _)| i)
                .unwrap_or(decoded_sequence.chars().count())..],
        );
    }
     */

    for detector in detectors {
        if detector.ratio() > 0.0 {
            trace!("{} produces ratio: {}", detector.name(), detector.ratio());
        }
    }
    trace!("===");

    mean_mess_ratio
}
