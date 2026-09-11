//! What the numbers in a Doom level mean.
//!
//! Three tables, kept apart from the layout in `wad.rs` because they are long
//! and because they are the part that is a fact about the game rather than
//! about the file: a thing type is an index into the executable's own list of
//! actors, and a linedef special is an index into its list of behaviours.
//!
//! None of the three is complete, and none of them has to be. A number nobody
//! named here is still read and still shown as itself; what a name adds is
//! that the reader does not have to look it up. The ones chosen are the ones a
//! level is built out of: every monster, every weapon, every pickup, the doors
//! and lifts and teleports, and the sector behaviours that have a name worth
//! saying. The obscure decorations and the two hundred remaining specials are
//! left as numbers on purpose, since a half-remembered name would be worse
//! than none.
//!
//! Doom's own names, as the game's source and its manuals use them, rather
//! than the nicknames the community settled on later: "Former human sergeant"
//! and not "Shotgun guy". A reader who knows only the nickname still has the
//! number, and a reader looking at the executable finds these.

/// What a thing is. The number is the actor's type, which is what a level
/// names and what the game looks up.
pub const THING_TYPES: &[(i128, &str)] = &[
    // Where players and monsters come in.
    (1, "Player 1 start"),
    (2, "Player 2 start"),
    (3, "Player 3 start"),
    (4, "Player 4 start"),
    (11, "Deathmatch start"),
    (14, "Teleport landing"),
    // Monsters.
    (3004, "Former human trooper"),
    (9, "Former human sergeant"),
    (65, "Heavy weapon dude"),
    (3001, "Imp"),
    (3002, "Demon"),
    (58, "Spectre"),
    (3006, "Lost soul"),
    (3005, "Cacodemon"),
    (69, "Hell knight"),
    (3003, "Baron of Hell"),
    (68, "Arachnotron"),
    (71, "Pain elemental"),
    (66, "Revenant"),
    (67, "Mancubus"),
    (64, "Arch-vile"),
    (7, "Spider mastermind"),
    (16, "Cyberdemon"),
    (84, "Wolfenstein SS"),
    (72, "Commander Keen"),
    // The three actors that run the final level rather than fight in it.
    (88, "Romero's head"),
    (89, "Monster spawner"),
    (87, "Monster spawn spot"),
    // Weapons.
    (2005, "Chainsaw"),
    (2001, "Shotgun"),
    (82, "Super shotgun"),
    (2002, "Chaingun"),
    (2003, "Rocket launcher"),
    (2004, "Plasma rifle"),
    (2006, "BFG9000"),
    // Ammunition.
    (2007, "Ammo clip"),
    (2048, "Box of ammo"),
    (2008, "4 shotgun shells"),
    (2049, "Box of shells"),
    (2010, "Rocket"),
    (2046, "Box of rockets"),
    (2047, "Energy cell"),
    (17, "Energy cell pack"),
    (8, "Backpack"),
    // Health and armour.
    (2011, "Stimpack"),
    (2012, "Medikit"),
    (2014, "Health potion"),
    (2015, "Spiritual armor"),
    (2018, "Armor"),
    (2019, "Megaarmor"),
    (2013, "Supercharge"),
    (83, "Megasphere"),
    // Powerups.
    (2022, "Invulnerability"),
    (2023, "Berserk"),
    (2024, "Partial invisibility"),
    (2025, "Radiation shielding suit"),
    (2026, "Computer area map"),
    (2045, "Light amplification visor"),
    // Keys. Six of them, three colours in two shapes, and a level may want
    // either shape for the same door.
    (5, "Blue keycard"),
    (40, "Blue skull key"),
    (6, "Yellow keycard"),
    (39, "Yellow skull key"),
    (13, "Red keycard"),
    (38, "Red skull key"),
    // The one decoration with a behaviour.
    (2035, "Exploding barrel"),
    // Light sources, which is most of what a level's remaining things are.
    (2028, "Floor lamp"),
    (34, "Candle"),
    (35, "Candelabra"),
    (85, "Tall techno floor lamp"),
    (86, "Short techno floor lamp"),
    (44, "Tall blue firestick"),
    (45, "Tall green firestick"),
    (46, "Tall red firestick"),
    (55, "Short blue firestick"),
    (56, "Short green firestick"),
    (57, "Short red firestick"),
    // Corpses and gore, which carry no behaviour but do block movement in
    // some cases, which is why they are worth naming.
    (10, "Bloody mess"),
    (12, "Bloody mess 2"),
    (15, "Dead player"),
    (18, "Dead former human"),
    (19, "Dead former sergeant"),
    (20, "Dead imp"),
    (21, "Dead demon"),
    (22, "Dead cacodemon"),
    (23, "Dead lost soul"),
    (24, "Pool of blood and flesh"),
    (25, "Impaled human"),
    (26, "Twitching impaled human"),
    (27, "Skull on a pole"),
    (28, "Five skulls shish kebab"),
    (29, "Pile of skulls and candles"),
    (30, "Tall green pillar"),
    (31, "Short green pillar"),
    (32, "Tall red pillar"),
    (33, "Short red pillar"),
    (36, "Short green pillar with beating heart"),
    (37, "Short red pillar with skull"),
    (41, "Evil eye"),
    (42, "Floating skull"),
    (43, "Burnt tree"),
    (47, "Stalagmite"),
    (48, "Tall techno pillar"),
    (49, "Hanging victim, twitching"),
    (50, "Hanging victim, arms out"),
    (51, "Hanging victim, one-legged"),
    (52, "Hanging pair of legs"),
    (53, "Hanging leg"),
    (54, "Large brown tree"),
    (59, "Hanging victim, arms out (no block)"),
    (60, "Hanging pair of legs (no block)"),
    (61, "Hanging victim, one-legged (no block)"),
    (62, "Hanging leg (no block)"),
    (63, "Hanging victim, twitching (no block)"),
    (70, "Burning barrel"),
    (73, "Hanging victim, guts removed"),
    (74, "Hanging victim, guts and brain removed"),
    (75, "Hanging torso, looking down"),
    (76, "Hanging torso, open skull"),
    (77, "Hanging torso, looking up"),
    (78, "Hanging torso, brain removed"),
    (79, "Pool of blood"),
    (80, "Pool of blood 2"),
    (81, "Pool of brains"),
];

/// What walking into, shooting or pressing a line does. The name says how it
/// is triggered as well as what it does, since a level's feel is mostly in
/// that difference: `W1` is walk over once, `WR` walk over repeatably, `S1`
/// switch once, `SR` switch repeatably, `G` shoot, and `D` is the manual one a
/// player pushes against.
///
/// The doors, lifts, stairs, teleports and level exits, which between them are
/// nearly every special a level actually uses. The remaining two hundred are
/// numbers here.
pub const LINE_SPECIALS: &[(i128, &str)] = &[
    (0, "None"),
    // Doors a player pushes against.
    (1, "D  Door, open and close"),
    (26, "D  Door, blue key, open and close"),
    (27, "D  Door, yellow key, open and close"),
    (28, "D  Door, red key, open and close"),
    (31, "D1 Door, open and stay"),
    (32, "D1 Door, blue key, open and stay"),
    (33, "D1 Door, red key, open and stay"),
    (34, "D1 Door, yellow key, open and stay"),
    (117, "D  Door, open and close fast"),
    (118, "D1 Door, open and stay, fast"),
    // Doors a line triggers.
    (2, "W1 Door, open and stay"),
    (3, "W1 Door, close"),
    (4, "W1 Door, open and close"),
    (16, "W1 Door, close and open after 30s"),
    (29, "S1 Door, open and close"),
    (42, "SR Door, close"),
    (46, "G  Door, open and stay"),
    (50, "S1 Door, close"),
    (61, "SR Door, open and stay"),
    (63, "SR Door, open and close"),
    (75, "WR Door, close"),
    (76, "WR Door, close and open after 30s"),
    (86, "WR Door, open and stay"),
    (90, "WR Door, open and close"),
    (99, "SR Door, blue key, open and stay, fast"),
    (103, "S1 Door, open and stay"),
    (133, "S1 Door, blue key, open and stay, fast"),
    (134, "SR Door, red key, open and stay, fast"),
    (135, "S1 Door, red key, open and stay, fast"),
    (136, "SR Door, yellow key, open and stay, fast"),
    (137, "S1 Door, yellow key, open and stay, fast"),
    // Lifts and floors.
    (10, "W1 Lift, lower, wait, raise"),
    (14, "S1 Floor, raise 32 and change texture"),
    (18, "S1 Floor, raise to next higher floor"),
    (19, "W1 Floor, lower to highest floor"),
    (21, "S1 Lift, lower, wait, raise"),
    (22, "W1 Floor, raise to next higher floor and change texture"),
    (23, "S1 Floor, lower to lowest floor"),
    (36, "W1 Floor, lower to 8 above highest floor"),
    (37, "W1 Floor, lower to lowest floor and change texture"),
    (38, "W1 Floor, lower to lowest floor"),
    (44, "W1 Ceiling, crush"),
    (45, "SR Floor, lower to highest floor"),
    (47, "G1 Floor, raise to next higher floor and change texture"),
    (53, "W1 Floor, start moving up and down"),
    (54, "W1 Floor, stop moving"),
    (58, "W1 Floor, raise 24"),
    (59, "W1 Floor, raise 24 and change texture"),
    (60, "SR Floor, lower to lowest floor"),
    (62, "SR Lift, lower, wait, raise"),
    (64, "SR Floor, raise to ceiling"),
    (66, "SR Floor, raise 24 and change texture"),
    (67, "SR Floor, raise 32 and change texture"),
    (68, "SR Floor, raise to next higher floor and change texture"),
    (69, "SR Floor, raise to next higher floor"),
    (70, "SR Floor, lower to 8 above highest floor"),
    (82, "WR Floor, lower to lowest floor"),
    (87, "WR Floor, start moving up and down"),
    (88, "WR Lift, lower, wait, raise"),
    (89, "WR Floor, stop moving"),
    (91, "WR Floor, raise to ceiling"),
    (92, "WR Floor, raise 24"),
    (93, "WR Floor, raise 24 and change texture"),
    (96, "WR Floor, raise by shortest lower texture"),
    (98, "WR Floor, lower to 8 above highest floor"),
    (101, "S1 Floor, raise to ceiling"),
    (102, "S1 Floor, lower to highest floor"),
    (119, "W1 Floor, raise to next higher floor"),
    (121, "W1 Lift, lower, wait, raise, fast"),
    (122, "S1 Lift, lower, wait, raise, fast"),
    (123, "SR Lift, lower, wait, raise, fast"),
    (128, "WR Floor, raise to next higher floor"),
    // Stairs.
    (7, "S1 Stairs, raise 8 a step"),
    (8, "W1 Stairs, raise 8 a step"),
    (100, "W1 Stairs, raise 16 a step, fast"),
    (127, "S1 Stairs, raise 16 a step, fast"),
    // Crushers.
    (6, "W1 Crusher, start, fast"),
    (25, "W1 Crusher, start"),
    (49, "S1 Ceiling, crush"),
    (57, "W1 Crusher, stop"),
    (73, "WR Crusher, start"),
    (74, "WR Crusher, stop"),
    (77, "WR Crusher, start, fast"),
    // Teleports.
    (39, "W1 Teleport"),
    (97, "WR Teleport"),
    (125, "W1 Teleport, monsters only"),
    (126, "WR Teleport, monsters only"),
    (174, "S1 Teleport"),
    (195, "SR Teleport"),
    // Level exits.
    (11, "S  Exit level"),
    (51, "S  Exit level, secret"),
    (52, "W  Exit level"),
    (124, "W  Exit level, secret"),
    (197, "G  Exit level"),
    (198, "G  Exit level, secret"),
    // Light.
    (12, "W1 Light, set to highest neighbour"),
    (13, "W1 Light, set to 255"),
    (17, "W1 Light, start blinking"),
    (35, "W1 Light, set to 35"),
    (79, "WR Light, set to 35"),
    (80, "WR Light, set to highest neighbour"),
    (81, "WR Light, set to 255"),
    (104, "W1 Light, set to lowest neighbour"),
    (138, "SR Light, set to 255"),
    (139, "SR Light, set to 35"),
    // Scrolling walls, which is how a level animates a texture.
    (48, "   Scroll wall left"),
    (85, "   Scroll wall right"),
];

/// What a sector does to the players in it, and to its own light.
///
/// Nearly complete: there are only sixteen of these in Doom, and a level uses
/// most of them.
pub const SECTOR_SPECIALS: &[(i128, &str)] = &[
    (0, "Normal"),
    (1, "Light blinks randomly"),
    (2, "Light blinks, 0.5s"),
    (3, "Light blinks, 1s"),
    (4, "Damage 20% a second, light blinks 0.5s"),
    (5, "Damage 10% a second"),
    (7, "Damage 5% a second"),
    (8, "Light oscillates"),
    (9, "Secret"),
    (10, "Door closes 30s after entry"),
    (11, "Damage 20% a second, ends level at 11% health"),
    (12, "Light blinks 1s, in step with neighbours"),
    (13, "Light blinks 0.5s, in step with neighbours"),
    (14, "Door opens 300s after entry"),
    (16, "Damage 20% a second"),
    (17, "Light flickers"),
];
