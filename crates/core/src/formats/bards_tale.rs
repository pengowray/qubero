//! The Bard's Tale I MS-DOS `.TPW` character and party files.
//!
//! Both kinds start with a sixteen-byte, NUL-padded name and a one-byte kind.
//! A character is 109 bytes long. A party is only a list of six sixteen-byte
//! character names, making its complete length 113 bytes. Multi-byte numbers
//! in character records are little-endian.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

const KIND: &[(i128, &str)] = &[(1, "character"), (2, "party")];
const STATUS: &[(u32, &str)] = &[
    (1, "unknown"),
    (2, "dead"),
    (3, "old"),
    (4, "poisoned"),
    (5, "stoned"),
    (6, "paralyzed"),
    (7, "possessed"),
];
const RACE: &[(i128, &str)] = &[
    (0, "human"),
    (1, "elf"),
    (2, "dwarf"),
    (3, "hobbit"),
    (4, "half-elf"),
    (5, "half-orc"),
    (6, "gnome"),
];
const CLASS: &[(i128, &str)] = &[
    (0, "warrior"),
    (1, "paladin"),
    (2, "rogue"),
    (3, "bard"),
    (4, "hunter"),
    (5, "monk"),
    (6, "conjurer"),
    (7, "magician"),
    (8, "sorcerer"),
    (9, "wizard"),
];
const ITEM_STATUS: &[(u32, &str)] = &[(6, "unidentified"), (7, "equipped")];
const ITEMS: &[(i128, &str)] = &[
    (0, "none"),
    (1, "torch"),
    (2, "lamp"),
    (3, "broadsword"),
    (4, "short sword"),
    (5, "dagger"),
    (6, "war axe"),
    (7, "halberd"),
    (8, "mace"),
    (9, "staff"),
    (10, "buckler"),
    (11, "tower shield"),
    (12, "leather armor"),
    (13, "chain mail"),
    (14, "scale armor"),
    (15, "plate armor"),
    (16, "robes"),
    (17, "helm"),
    (18, "leather gloves"),
    (19, "gauntlets"),
    (20, "mandolin"),
    (21, "harp"),
    (22, "flute"),
    (23, "mithril sword"),
    (24, "mithril shield"),
    (25, "mithril chain"),
    (26, "mithril scale"),
    (27, "samurai figurine"),
    (28, "bracers [6]"),
    (29, "bardsword"),
    (30, "fire horn"),
    (31, "lightwand"),
    (32, "mithril dagger"),
    (33, "mithril helm"),
    (34, "mithril gloves"),
    (35, "mithril axe"),
    (36, "mithril mace"),
    (37, "mithril plate"),
    (38, "ogre figurine"),
    (39, "Lak's Lyre"),
    (40, "shield ring"),
    (41, "Dork Ring"),
    (42, "Fin's Flute"),
    (43, "Kael's Axe"),
    (44, "blood axe"),
    (45, "dayblade"),
    (46, "shield staff"),
    (47, "elf cloak"),
    (48, "hawkblade"),
    (49, "adamant sword"),
    (50, "adamant shield"),
    (51, "adamant dagger"),
    (52, "adamant helm"),
    (53, "adamant gloves"),
    (54, "adamant mace"),
    (55, "broom"),
    (56, "pureblade"),
    (57, "exorwand"),
    (58, "Ali's Carpet"),
    (59, "magic mouth"),
    (60, "luckshield"),
    (61, "giant figurine"),
    (62, "adamant chain"),
    (63, "adamant scale"),
    (64, "adamant plate"),
    (65, "bracers [4]"),
    (66, "arcshield"),
    (67, "pure shield"),
    (68, "mage staff"),
    (69, "war staff"),
    (70, "thief dagger"),
    (71, "soul mace"),
    (72, "wither staff"),
    (73, "sorcerstaff"),
    (74, "sword of Pak"),
    (75, "heal harp"),
    (76, "Galt's Flute"),
    (77, "frost horn"),
    (78, "diamond sword"),
    (79, "diamond shield"),
    (80, "diamond dagger"),
    (81, "diamond helm"),
    (82, "golem figurine"),
    (83, "titan figurine"),
    (84, "conjurstaff"),
    (85, "Arc's Hammer"),
    (86, "staff of Lor"),
    (87, "powerstaff"),
    (88, "mournblade"),
    (89, "dragonshield"),
    (90, "diamond plate"),
    (91, "wargloves"),
    (92, "lorehelm"),
    (93, "dragonwand"),
    (94, "Kiel's Compass"),
    (95, "speedboots"),
    (96, "flame horn"),
    (97, "truthdrum"),
    (98, "spiritdrum"),
    (99, "pipes of Pan"),
    (100, "ring of power"),
    (101, "deathring"),
    (102, "Ybarra shield"),
    (103, "spectre mace"),
    (104, "Dag Stone"),
    (105, "Arc's Eye"),
    (106, "ogrewand"),
    (107, "spirithelm"),
    (108, "dragon figurine"),
    (109, "mage figurine"),
    (110, "troll ring"),
    (111, "troll staff"),
    (112, "onyx key"),
    (113, "crystal sword"),
    (114, "stoneblade"),
    (115, "travelhelm"),
    (116, "death dagger"),
    (117, "mongo figurine"),
    (118, "lich figurine"),
    (119, "eye"),
    (120, "master key"),
    (121, "wizwand"),
    (122, "silver square"),
    (123, "silver circle"),
    (124, "silver triangle"),
    (125, "Thor figurine"),
    (126, "old man figurine"),
    (127, "spectre snare"),
];

pub fn bards_tale() -> Template {
    Template::new(
        "bardstale",
        T::structure_named(
            "BardsTaleSave",
            "name",
            "contents",
            vec![
                ("name", name()),
                ("kind", T::enumeration("SaveKind", T::u8(), KIND)),
                (
                    "contents",
                    T::switch(
                        E::field("kind"),
                        vec![(1, character()), (2, party())],
                        T::bytes(E::Remaining),
                    ),
                ),
            ],
        ),
    )
}

fn name() -> T {
    T::text(
        StrLen::Padded {
            size: E::lit(16),
            pad: 0,
        },
        Encoding::Ascii,
    )
}

fn character() -> T {
    T::structure(
        "Character",
        vec![
            ("status", T::flags("Status", T::u16(Little), STATUS)),
            ("race", T::enumeration("Race", T::u16(Little), RACE)),
            ("class", T::enumeration("Class", T::u16(Little), CLASS)),
            ("current_strength", T::u16(Little)),
            ("current_iq", T::u16(Little)),
            ("current_dexterity", T::u16(Little)),
            ("current_constitution", T::u16(Little)),
            ("current_luck", T::u16(Little)),
            ("maximum_strength", T::u16(Little)),
            ("maximum_iq", T::u16(Little)),
            ("maximum_dexterity", T::u16(Little)),
            ("maximum_constitution", T::u16(Little)),
            ("maximum_luck", T::u16(Little)),
            ("base_armour_class", T::u16(Little)),
            ("maximum_hit_points", T::u16(Little)),
            ("current_hit_points", T::u16(Little)),
            ("maximum_spell_points", T::u16(Little)),
            ("current_spell_points", T::u16(Little)),
            ("inventory", T::array(item(), E::lit(8)).counted_as("items")),
            ("experience", T::u32(Little)),
            ("gold", T::u32(Little)),
            ("current_level", T::u16(Little)),
            ("maximum_level", T::u16(Little)),
            ("sorcerer_spell_level", T::u8()),
            ("conjurer_spell_level", T::u8()),
            ("magician_spell_level", T::u8()),
            ("wizard_spell_level", T::u8()),
            ("rogue_hide_chance", T::u8()),
            ("unknown_69", T::bytes(E::lit(5))),
            ("hunter_critical_chance", T::u16(Little)),
            ("bard_songs", T::u16(Little)),
            ("unknown_78", T::bytes(E::lit(6))),
            ("attacks_per_round", T::u16(Little)),
            ("unknown_86", T::bytes(E::lit(2))),
            ("battles_survived", T::u16(Little)),
            ("unknown_90", T::bytes(E::lit(2))),
        ],
    )
}

fn item() -> T {
    T::inline_structure(
        "Item",
        vec![
            ("index", T::enumeration("Item", T::u8(), ITEMS)),
            ("status", T::flags("ItemStatus", T::u8(), ITEM_STATUS)),
        ],
    )
}

fn party() -> T {
    T::structure(
        "Party",
        vec![("members", T::array(name(), E::lit(6)).counted_as("members"))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn header(name: &[u8], kind: u8) -> Vec<u8> {
        let mut v = vec![0; 16];
        v[..name.len()].copy_from_slice(name);
        v.push(kind);
        v
    }

    #[test]
    fn character_fields_land_at_the_documented_offsets() {
        let mut v = header(b"MERLIN", 1);
        v.resize(109, 0);
        v[19] = 6;
        v[21] = 9;
        v[45..47].copy_from_slice(&320u16.to_le_bytes());
        v[53..55].copy_from_slice(&[0x5b, 0x80]);
        v[69..73].copy_from_slice(&1_056_816u32.to_le_bytes());
        v[91..93].copy_from_slice(&255u16.to_le_bytes());
        v[93..95].copy_from_slice(&6u16.to_le_bytes());
        v[101..103].copy_from_slice(&3u16.to_le_bytes());
        v[105..107].copy_from_slice(&22u16.to_le_bytes());

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bards_tale());
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Str("MERLIN".into())
        );
        assert_eq!(
            ev.node(&d, &[2, 1]).unwrap().value,
            Value::Enum {
                raw: 6,
                name: Some("gnome".into()),
                hex: false
            }
        );
        assert_eq!(
            ev.node(&d, &[2, 2]).unwrap().value,
            Value::Enum {
                raw: 9,
                name: Some("wizard".into()),
                hex: false
            }
        );
        assert_eq!(ev.node(&d, &[2, 14]).unwrap().value, Value::UInt(320));
        assert_eq!(
            ev.node(&d, &[2, 18, 0, 0]).unwrap().value,
            Value::Enum {
                raw: 0x5b,
                name: Some("wargloves".into()),
                hex: false
            }
        );
        assert_eq!(ev.node(&d, &[2, 19]).unwrap().value, Value::UInt(1_056_816));
        assert_eq!(ev.node(&d, &[2, 29]).unwrap().value, Value::UInt(255));
        assert_eq!(ev.node(&d, &[2, 30]).unwrap().value, Value::UInt(6));
        assert_eq!(ev.node(&d, &[2, 32]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[2, 34]).unwrap().value, Value::UInt(22));
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 92 * 8);
    }

    #[test]
    fn a_party_contains_six_names() {
        let mut v = header(b"A-TEAM", 2);
        for member in [
            b"BRIAN".as_slice(),
            b"EL CID",
            b"MARKUS",
            b"MERLIN",
            b"OMAR",
            b"TARJAN",
        ] {
            let mut slot = [0u8; 16];
            slot[..member.len()].copy_from_slice(member);
            v.extend_from_slice(&slot);
        }
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bards_tale());
        assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().child_count, 6);
        assert_eq!(
            ev.node(&d, &[2, 0, 3]).unwrap().value,
            Value::Str("MERLIN".into())
        );
    }

    #[test]
    fn tpw_records_are_sniffed_without_claiming_lookalikes() {
        let mut character = header(b"BRIAN THE FIST", 1);
        character.resize(109, 0);
        character[19] = 0;
        character[21] = 1;
        assert_eq!(
            crate::formats::sniff(&character, character.len() as u64),
            Some("bardstale")
        );

        let mut party = header(b"A-TEAM", 2);
        party.resize(113, 0);
        assert_eq!(
            crate::formats::sniff(&party, party.len() as u64),
            Some("bardstale")
        );

        character[19] = 7; // not a race the game defines
        assert_eq!(
            crate::formats::sniff(&character, character.len() as u64),
            None
        );
        assert_eq!(crate::formats::sniff(b"ordinary text", 109), None);
    }
}
