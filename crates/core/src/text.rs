//! Encodings for text fields.
//!
//! Hand-rolled rather than pulled in: `encoding_rs` carries the whole WHATWG
//! set (weight this never needs in a wasm bundle) and still does not have
//! CP437, which a hex editor meets constantly in DOS-era formats. What is here
//! is UTF-8, ASCII, UTF-16 either way round and a table per single-byte code
//! page, with a BOM sniffer and a guess, each a few lines.

use crate::template::{Encoding, Endian};

// The high half of each single-byte code page, generated from Python's codecs
// so no table here is typed from memory. Below 0x80 every one of them is
// ASCII. U+FFFD marks a byte the page leaves undefined, which is how a page
// comes to refuse bytes the way the other encodings do.

const LATIN1_HIGH: [char; 128] = [
    '\u{0080}', '\u{0081}', '\u{0082}', '\u{0083}', '\u{0084}', '\u{0085}', '\u{0086}', '\u{0087}',
    '\u{0088}', '\u{0089}', '\u{008a}', '\u{008b}', '\u{008c}', '\u{008d}', '\u{008e}', '\u{008f}',
    '\u{0090}', '\u{0091}', '\u{0092}', '\u{0093}', '\u{0094}', '\u{0095}', '\u{0096}', '\u{0097}',
    '\u{0098}', '\u{0099}', '\u{009a}', '\u{009b}', '\u{009c}', '\u{009d}', '\u{009e}', '\u{009f}',
    '\u{00a0}', '\u{00a1}', '\u{00a2}', '\u{00a3}', '\u{00a4}', '\u{00a5}', '\u{00a6}', '\u{00a7}',
    '\u{00a8}', '\u{00a9}', '\u{00aa}', '\u{00ab}', '\u{00ac}', '\u{00ad}', '\u{00ae}', '\u{00af}',
    '\u{00b0}', '\u{00b1}', '\u{00b2}', '\u{00b3}', '\u{00b4}', '\u{00b5}', '\u{00b6}', '\u{00b7}',
    '\u{00b8}', '\u{00b9}', '\u{00ba}', '\u{00bb}', '\u{00bc}', '\u{00bd}', '\u{00be}', '\u{00bf}',
    '\u{00c0}', '\u{00c1}', '\u{00c2}', '\u{00c3}', '\u{00c4}', '\u{00c5}', '\u{00c6}', '\u{00c7}',
    '\u{00c8}', '\u{00c9}', '\u{00ca}', '\u{00cb}', '\u{00cc}', '\u{00cd}', '\u{00ce}', '\u{00cf}',
    '\u{00d0}', '\u{00d1}', '\u{00d2}', '\u{00d3}', '\u{00d4}', '\u{00d5}', '\u{00d6}', '\u{00d7}',
    '\u{00d8}', '\u{00d9}', '\u{00da}', '\u{00db}', '\u{00dc}', '\u{00dd}', '\u{00de}', '\u{00df}',
    '\u{00e0}', '\u{00e1}', '\u{00e2}', '\u{00e3}', '\u{00e4}', '\u{00e5}', '\u{00e6}', '\u{00e7}',
    '\u{00e8}', '\u{00e9}', '\u{00ea}', '\u{00eb}', '\u{00ec}', '\u{00ed}', '\u{00ee}', '\u{00ef}',
    '\u{00f0}', '\u{00f1}', '\u{00f2}', '\u{00f3}', '\u{00f4}', '\u{00f5}', '\u{00f6}', '\u{00f7}',
    '\u{00f8}', '\u{00f9}', '\u{00fa}', '\u{00fb}', '\u{00fc}', '\u{00fd}', '\u{00fe}', '\u{00ff}',
];

const WINDOWS1252_HIGH: [char; 128] = [
    '\u{20ac}', '\u{fffd}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{fffd}', '\u{017d}', '\u{fffd}',
    '\u{fffd}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}', '\u{0153}', '\u{fffd}', '\u{017e}', '\u{0178}',
    '\u{00a0}', '\u{00a1}', '\u{00a2}', '\u{00a3}', '\u{00a4}', '\u{00a5}', '\u{00a6}', '\u{00a7}',
    '\u{00a8}', '\u{00a9}', '\u{00aa}', '\u{00ab}', '\u{00ac}', '\u{00ad}', '\u{00ae}', '\u{00af}',
    '\u{00b0}', '\u{00b1}', '\u{00b2}', '\u{00b3}', '\u{00b4}', '\u{00b5}', '\u{00b6}', '\u{00b7}',
    '\u{00b8}', '\u{00b9}', '\u{00ba}', '\u{00bb}', '\u{00bc}', '\u{00bd}', '\u{00be}', '\u{00bf}',
    '\u{00c0}', '\u{00c1}', '\u{00c2}', '\u{00c3}', '\u{00c4}', '\u{00c5}', '\u{00c6}', '\u{00c7}',
    '\u{00c8}', '\u{00c9}', '\u{00ca}', '\u{00cb}', '\u{00cc}', '\u{00cd}', '\u{00ce}', '\u{00cf}',
    '\u{00d0}', '\u{00d1}', '\u{00d2}', '\u{00d3}', '\u{00d4}', '\u{00d5}', '\u{00d6}', '\u{00d7}',
    '\u{00d8}', '\u{00d9}', '\u{00da}', '\u{00db}', '\u{00dc}', '\u{00dd}', '\u{00de}', '\u{00df}',
    '\u{00e0}', '\u{00e1}', '\u{00e2}', '\u{00e3}', '\u{00e4}', '\u{00e5}', '\u{00e6}', '\u{00e7}',
    '\u{00e8}', '\u{00e9}', '\u{00ea}', '\u{00eb}', '\u{00ec}', '\u{00ed}', '\u{00ee}', '\u{00ef}',
    '\u{00f0}', '\u{00f1}', '\u{00f2}', '\u{00f3}', '\u{00f4}', '\u{00f5}', '\u{00f6}', '\u{00f7}',
    '\u{00f8}', '\u{00f9}', '\u{00fa}', '\u{00fb}', '\u{00fc}', '\u{00fd}', '\u{00fe}', '\u{00ff}',
];

const ISO8859_15_HIGH: [char; 128] = [
    '\u{0080}', '\u{0081}', '\u{0082}', '\u{0083}', '\u{0084}', '\u{0085}', '\u{0086}', '\u{0087}',
    '\u{0088}', '\u{0089}', '\u{008a}', '\u{008b}', '\u{008c}', '\u{008d}', '\u{008e}', '\u{008f}',
    '\u{0090}', '\u{0091}', '\u{0092}', '\u{0093}', '\u{0094}', '\u{0095}', '\u{0096}', '\u{0097}',
    '\u{0098}', '\u{0099}', '\u{009a}', '\u{009b}', '\u{009c}', '\u{009d}', '\u{009e}', '\u{009f}',
    '\u{00a0}', '\u{00a1}', '\u{00a2}', '\u{00a3}', '\u{20ac}', '\u{00a5}', '\u{0160}', '\u{00a7}',
    '\u{0161}', '\u{00a9}', '\u{00aa}', '\u{00ab}', '\u{00ac}', '\u{00ad}', '\u{00ae}', '\u{00af}',
    '\u{00b0}', '\u{00b1}', '\u{00b2}', '\u{00b3}', '\u{017d}', '\u{00b5}', '\u{00b6}', '\u{00b7}',
    '\u{017e}', '\u{00b9}', '\u{00ba}', '\u{00bb}', '\u{0152}', '\u{0153}', '\u{0178}', '\u{00bf}',
    '\u{00c0}', '\u{00c1}', '\u{00c2}', '\u{00c3}', '\u{00c4}', '\u{00c5}', '\u{00c6}', '\u{00c7}',
    '\u{00c8}', '\u{00c9}', '\u{00ca}', '\u{00cb}', '\u{00cc}', '\u{00cd}', '\u{00ce}', '\u{00cf}',
    '\u{00d0}', '\u{00d1}', '\u{00d2}', '\u{00d3}', '\u{00d4}', '\u{00d5}', '\u{00d6}', '\u{00d7}',
    '\u{00d8}', '\u{00d9}', '\u{00da}', '\u{00db}', '\u{00dc}', '\u{00dd}', '\u{00de}', '\u{00df}',
    '\u{00e0}', '\u{00e1}', '\u{00e2}', '\u{00e3}', '\u{00e4}', '\u{00e5}', '\u{00e6}', '\u{00e7}',
    '\u{00e8}', '\u{00e9}', '\u{00ea}', '\u{00eb}', '\u{00ec}', '\u{00ed}', '\u{00ee}', '\u{00ef}',
    '\u{00f0}', '\u{00f1}', '\u{00f2}', '\u{00f3}', '\u{00f4}', '\u{00f5}', '\u{00f6}', '\u{00f7}',
    '\u{00f8}', '\u{00f9}', '\u{00fa}', '\u{00fb}', '\u{00fc}', '\u{00fd}', '\u{00fe}', '\u{00ff}',
];

const ISO8859_2_HIGH: [char; 128] = [
    '\u{0080}', '\u{0081}', '\u{0082}', '\u{0083}', '\u{0084}', '\u{0085}', '\u{0086}', '\u{0087}',
    '\u{0088}', '\u{0089}', '\u{008a}', '\u{008b}', '\u{008c}', '\u{008d}', '\u{008e}', '\u{008f}',
    '\u{0090}', '\u{0091}', '\u{0092}', '\u{0093}', '\u{0094}', '\u{0095}', '\u{0096}', '\u{0097}',
    '\u{0098}', '\u{0099}', '\u{009a}', '\u{009b}', '\u{009c}', '\u{009d}', '\u{009e}', '\u{009f}',
    '\u{00a0}', '\u{0104}', '\u{02d8}', '\u{0141}', '\u{00a4}', '\u{013d}', '\u{015a}', '\u{00a7}',
    '\u{00a8}', '\u{0160}', '\u{015e}', '\u{0164}', '\u{0179}', '\u{00ad}', '\u{017d}', '\u{017b}',
    '\u{00b0}', '\u{0105}', '\u{02db}', '\u{0142}', '\u{00b4}', '\u{013e}', '\u{015b}', '\u{02c7}',
    '\u{00b8}', '\u{0161}', '\u{015f}', '\u{0165}', '\u{017a}', '\u{02dd}', '\u{017e}', '\u{017c}',
    '\u{0154}', '\u{00c1}', '\u{00c2}', '\u{0102}', '\u{00c4}', '\u{0139}', '\u{0106}', '\u{00c7}',
    '\u{010c}', '\u{00c9}', '\u{0118}', '\u{00cb}', '\u{011a}', '\u{00cd}', '\u{00ce}', '\u{010e}',
    '\u{0110}', '\u{0143}', '\u{0147}', '\u{00d3}', '\u{00d4}', '\u{0150}', '\u{00d6}', '\u{00d7}',
    '\u{0158}', '\u{016e}', '\u{00da}', '\u{0170}', '\u{00dc}', '\u{00dd}', '\u{0162}', '\u{00df}',
    '\u{0155}', '\u{00e1}', '\u{00e2}', '\u{0103}', '\u{00e4}', '\u{013a}', '\u{0107}', '\u{00e7}',
    '\u{010d}', '\u{00e9}', '\u{0119}', '\u{00eb}', '\u{011b}', '\u{00ed}', '\u{00ee}', '\u{010f}',
    '\u{0111}', '\u{0144}', '\u{0148}', '\u{00f3}', '\u{00f4}', '\u{0151}', '\u{00f6}', '\u{00f7}',
    '\u{0159}', '\u{016f}', '\u{00fa}', '\u{0171}', '\u{00fc}', '\u{00fd}', '\u{0163}', '\u{02d9}',
];

const WINDOWS1250_HIGH: [char; 128] = [
    '\u{20ac}', '\u{fffd}', '\u{201a}', '\u{fffd}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{fffd}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{015a}', '\u{0164}', '\u{017d}', '\u{0179}',
    '\u{fffd}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{fffd}', '\u{2122}', '\u{0161}', '\u{203a}', '\u{015b}', '\u{0165}', '\u{017e}', '\u{017a}',
    '\u{00a0}', '\u{02c7}', '\u{02d8}', '\u{0141}', '\u{00a4}', '\u{0104}', '\u{00a6}', '\u{00a7}',
    '\u{00a8}', '\u{00a9}', '\u{015e}', '\u{00ab}', '\u{00ac}', '\u{00ad}', '\u{00ae}', '\u{017b}',
    '\u{00b0}', '\u{00b1}', '\u{02db}', '\u{0142}', '\u{00b4}', '\u{00b5}', '\u{00b6}', '\u{00b7}',
    '\u{00b8}', '\u{0105}', '\u{015f}', '\u{00bb}', '\u{013d}', '\u{02dd}', '\u{013e}', '\u{017c}',
    '\u{0154}', '\u{00c1}', '\u{00c2}', '\u{0102}', '\u{00c4}', '\u{0139}', '\u{0106}', '\u{00c7}',
    '\u{010c}', '\u{00c9}', '\u{0118}', '\u{00cb}', '\u{011a}', '\u{00cd}', '\u{00ce}', '\u{010e}',
    '\u{0110}', '\u{0143}', '\u{0147}', '\u{00d3}', '\u{00d4}', '\u{0150}', '\u{00d6}', '\u{00d7}',
    '\u{0158}', '\u{016e}', '\u{00da}', '\u{0170}', '\u{00dc}', '\u{00dd}', '\u{0162}', '\u{00df}',
    '\u{0155}', '\u{00e1}', '\u{00e2}', '\u{0103}', '\u{00e4}', '\u{013a}', '\u{0107}', '\u{00e7}',
    '\u{010d}', '\u{00e9}', '\u{0119}', '\u{00eb}', '\u{011b}', '\u{00ed}', '\u{00ee}', '\u{010f}',
    '\u{0111}', '\u{0144}', '\u{0148}', '\u{00f3}', '\u{00f4}', '\u{0151}', '\u{00f6}', '\u{00f7}',
    '\u{0159}', '\u{016f}', '\u{00fa}', '\u{0171}', '\u{00fc}', '\u{00fd}', '\u{0163}', '\u{02d9}',
];

const WINDOWS1251_HIGH: [char; 128] = [
    '\u{0402}', '\u{0403}', '\u{201a}', '\u{0453}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{20ac}', '\u{2030}', '\u{0409}', '\u{2039}', '\u{040a}', '\u{040c}', '\u{040b}', '\u{040f}',
    '\u{0452}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{fffd}', '\u{2122}', '\u{0459}', '\u{203a}', '\u{045a}', '\u{045c}', '\u{045b}', '\u{045f}',
    '\u{00a0}', '\u{040e}', '\u{045e}', '\u{0408}', '\u{00a4}', '\u{0490}', '\u{00a6}', '\u{00a7}',
    '\u{0401}', '\u{00a9}', '\u{0404}', '\u{00ab}', '\u{00ac}', '\u{00ad}', '\u{00ae}', '\u{0407}',
    '\u{00b0}', '\u{00b1}', '\u{0406}', '\u{0456}', '\u{0491}', '\u{00b5}', '\u{00b6}', '\u{00b7}',
    '\u{0451}', '\u{2116}', '\u{0454}', '\u{00bb}', '\u{0458}', '\u{0405}', '\u{0455}', '\u{0457}',
    '\u{0410}', '\u{0411}', '\u{0412}', '\u{0413}', '\u{0414}', '\u{0415}', '\u{0416}', '\u{0417}',
    '\u{0418}', '\u{0419}', '\u{041a}', '\u{041b}', '\u{041c}', '\u{041d}', '\u{041e}', '\u{041f}',
    '\u{0420}', '\u{0421}', '\u{0422}', '\u{0423}', '\u{0424}', '\u{0425}', '\u{0426}', '\u{0427}',
    '\u{0428}', '\u{0429}', '\u{042a}', '\u{042b}', '\u{042c}', '\u{042d}', '\u{042e}', '\u{042f}',
    '\u{0430}', '\u{0431}', '\u{0432}', '\u{0433}', '\u{0434}', '\u{0435}', '\u{0436}', '\u{0437}',
    '\u{0438}', '\u{0439}', '\u{043a}', '\u{043b}', '\u{043c}', '\u{043d}', '\u{043e}', '\u{043f}',
    '\u{0440}', '\u{0441}', '\u{0442}', '\u{0443}', '\u{0444}', '\u{0445}', '\u{0446}', '\u{0447}',
    '\u{0448}', '\u{0449}', '\u{044a}', '\u{044b}', '\u{044c}', '\u{044d}', '\u{044e}', '\u{044f}',
];

const KOI8R_HIGH: [char; 128] = [
    '\u{2500}', '\u{2502}', '\u{250c}', '\u{2510}', '\u{2514}', '\u{2518}', '\u{251c}', '\u{2524}',
    '\u{252c}', '\u{2534}', '\u{253c}', '\u{2580}', '\u{2584}', '\u{2588}', '\u{258c}', '\u{2590}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2320}', '\u{25a0}', '\u{2219}', '\u{221a}', '\u{2248}',
    '\u{2264}', '\u{2265}', '\u{00a0}', '\u{2321}', '\u{00b0}', '\u{00b2}', '\u{00b7}', '\u{00f7}',
    '\u{2550}', '\u{2551}', '\u{2552}', '\u{0451}', '\u{2553}', '\u{2554}', '\u{2555}', '\u{2556}',
    '\u{2557}', '\u{2558}', '\u{2559}', '\u{255a}', '\u{255b}', '\u{255c}', '\u{255d}', '\u{255e}',
    '\u{255f}', '\u{2560}', '\u{2561}', '\u{0401}', '\u{2562}', '\u{2563}', '\u{2564}', '\u{2565}',
    '\u{2566}', '\u{2567}', '\u{2568}', '\u{2569}', '\u{256a}', '\u{256b}', '\u{256c}', '\u{00a9}',
    '\u{044e}', '\u{0430}', '\u{0431}', '\u{0446}', '\u{0434}', '\u{0435}', '\u{0444}', '\u{0433}',
    '\u{0445}', '\u{0438}', '\u{0439}', '\u{043a}', '\u{043b}', '\u{043c}', '\u{043d}', '\u{043e}',
    '\u{043f}', '\u{044f}', '\u{0440}', '\u{0441}', '\u{0442}', '\u{0443}', '\u{0436}', '\u{0432}',
    '\u{044c}', '\u{044b}', '\u{0437}', '\u{0448}', '\u{044d}', '\u{0449}', '\u{0447}', '\u{044a}',
    '\u{042e}', '\u{0410}', '\u{0411}', '\u{0426}', '\u{0414}', '\u{0415}', '\u{0424}', '\u{0413}',
    '\u{0425}', '\u{0418}', '\u{0419}', '\u{041a}', '\u{041b}', '\u{041c}', '\u{041d}', '\u{041e}',
    '\u{041f}', '\u{042f}', '\u{0420}', '\u{0421}', '\u{0422}', '\u{0423}', '\u{0416}', '\u{0412}',
    '\u{042c}', '\u{042b}', '\u{0417}', '\u{0428}', '\u{042d}', '\u{0429}', '\u{0427}', '\u{042a}',
];

const MACROMAN_HIGH: [char; 128] = [
    '\u{00c4}', '\u{00c5}', '\u{00c7}', '\u{00c9}', '\u{00d1}', '\u{00d6}', '\u{00dc}', '\u{00e1}',
    '\u{00e0}', '\u{00e2}', '\u{00e4}', '\u{00e3}', '\u{00e5}', '\u{00e7}', '\u{00e9}', '\u{00e8}',
    '\u{00ea}', '\u{00eb}', '\u{00ed}', '\u{00ec}', '\u{00ee}', '\u{00ef}', '\u{00f1}', '\u{00f3}',
    '\u{00f2}', '\u{00f4}', '\u{00f6}', '\u{00f5}', '\u{00fa}', '\u{00f9}', '\u{00fb}', '\u{00fc}',
    '\u{2020}', '\u{00b0}', '\u{00a2}', '\u{00a3}', '\u{00a7}', '\u{2022}', '\u{00b6}', '\u{00df}',
    '\u{00ae}', '\u{00a9}', '\u{2122}', '\u{00b4}', '\u{00a8}', '\u{2260}', '\u{00c6}', '\u{00d8}',
    '\u{221e}', '\u{00b1}', '\u{2264}', '\u{2265}', '\u{00a5}', '\u{00b5}', '\u{2202}', '\u{2211}',
    '\u{220f}', '\u{03c0}', '\u{222b}', '\u{00aa}', '\u{00ba}', '\u{03a9}', '\u{00e6}', '\u{00f8}',
    '\u{00bf}', '\u{00a1}', '\u{00ac}', '\u{221a}', '\u{0192}', '\u{2248}', '\u{2206}', '\u{00ab}',
    '\u{00bb}', '\u{2026}', '\u{00a0}', '\u{00c0}', '\u{00c3}', '\u{00d5}', '\u{0152}', '\u{0153}',
    '\u{2013}', '\u{2014}', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '\u{00f7}', '\u{25ca}',
    '\u{00ff}', '\u{0178}', '\u{2044}', '\u{20ac}', '\u{2039}', '\u{203a}', '\u{fb01}', '\u{fb02}',
    '\u{2021}', '\u{00b7}', '\u{201a}', '\u{201e}', '\u{2030}', '\u{00c2}', '\u{00ca}', '\u{00c1}',
    '\u{00cb}', '\u{00c8}', '\u{00cd}', '\u{00ce}', '\u{00cf}', '\u{00cc}', '\u{00d3}', '\u{00d4}',
    '\u{f8ff}', '\u{00d2}', '\u{00da}', '\u{00db}', '\u{00d9}', '\u{0131}', '\u{02c6}', '\u{02dc}',
    '\u{00af}', '\u{02d8}', '\u{02d9}', '\u{02da}', '\u{00b8}', '\u{02dd}', '\u{02db}', '\u{02c7}',
];

const CP437_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00a2}', '\u{00a3}', '\u{00a5}', '\u{20a7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{2310}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{03b1}', '\u{00df}', '\u{0393}', '\u{03c0}', '\u{03a3}', '\u{03c3}', '\u{00b5}', '\u{03c4}',
    '\u{03a6}', '\u{0398}', '\u{03a9}', '\u{03b4}', '\u{221e}', '\u{03c6}', '\u{03b5}', '\u{2229}',
    '\u{2261}', '\u{00b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00f7}', '\u{2248}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{207f}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

const CP850_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00f8}', '\u{00a3}', '\u{00d8}', '\u{00d7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{00ae}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{00c1}', '\u{00c2}', '\u{00c0}',
    '\u{00a9}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{00a2}', '\u{00a5}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{00e3}', '\u{00c3}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{00a4}',
    '\u{00f0}', '\u{00d0}', '\u{00ca}', '\u{00cb}', '\u{00c8}', '\u{0131}', '\u{00cd}', '\u{00ce}',
    '\u{00cf}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{00a6}', '\u{00cc}', '\u{2580}',
    '\u{00d3}', '\u{00df}', '\u{00d4}', '\u{00d2}', '\u{00f5}', '\u{00d5}', '\u{00b5}', '\u{00fe}',
    '\u{00de}', '\u{00da}', '\u{00db}', '\u{00d9}', '\u{00fd}', '\u{00dd}', '\u{00af}', '\u{00b4}',
    '\u{00ad}', '\u{00b1}', '\u{2017}', '\u{00be}', '\u{00b6}', '\u{00a7}', '\u{00f7}', '\u{00b8}',
    '\u{00b0}', '\u{00a8}', '\u{00b7}', '\u{00b9}', '\u{00b3}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

const CP866_HIGH: [char; 128] = [
    '\u{0410}', '\u{0411}', '\u{0412}', '\u{0413}', '\u{0414}', '\u{0415}', '\u{0416}', '\u{0417}',
    '\u{0418}', '\u{0419}', '\u{041a}', '\u{041b}', '\u{041c}', '\u{041d}', '\u{041e}', '\u{041f}',
    '\u{0420}', '\u{0421}', '\u{0422}', '\u{0423}', '\u{0424}', '\u{0425}', '\u{0426}', '\u{0427}',
    '\u{0428}', '\u{0429}', '\u{042a}', '\u{042b}', '\u{042c}', '\u{042d}', '\u{042e}', '\u{042f}',
    '\u{0430}', '\u{0431}', '\u{0432}', '\u{0433}', '\u{0434}', '\u{0435}', '\u{0436}', '\u{0437}',
    '\u{0438}', '\u{0439}', '\u{043a}', '\u{043b}', '\u{043c}', '\u{043d}', '\u{043e}', '\u{043f}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{0440}', '\u{0441}', '\u{0442}', '\u{0443}', '\u{0444}', '\u{0445}', '\u{0446}', '\u{0447}',
    '\u{0448}', '\u{0449}', '\u{044a}', '\u{044b}', '\u{044c}', '\u{044d}', '\u{044e}', '\u{044f}',
    '\u{0401}', '\u{0451}', '\u{0404}', '\u{0454}', '\u{0407}', '\u{0457}', '\u{040e}', '\u{045e}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{2116}', '\u{00a4}', '\u{25a0}', '\u{00a0}',
];
/// A single-byte code page: ASCII below 0x80, a table of 128 characters above
/// it. Which one a file is in is never in the bytes, so it is a choice the
/// reader makes, and the two slots the panel offers are one from each family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePage {
    Latin1,
    Windows1252,
    Iso8859_15,
    Iso8859_2,
    Windows1250,
    Windows1251,
    Koi8R,
    MacRoman,
    Cp437,
    Cp850,
    Cp866,
}

impl CodePage {
    /// The pages of the ISO, Windows, Mac and KOI8 family, which is where a
    /// file that is not from DOS is likely to be.
    pub const SLOT_A: [CodePage; 8] = [
        CodePage::Latin1,
        CodePage::Windows1252,
        CodePage::Iso8859_15,
        CodePage::Iso8859_2,
        CodePage::Windows1250,
        CodePage::Windows1251,
        CodePage::Koi8R,
        CodePage::MacRoman,
    ];

    /// The DOS pages, which is where a file with box drawing in it is.
    pub const SLOT_B: [CodePage; 3] = [CodePage::Cp437, CodePage::Cp850, CodePage::Cp866];

    pub fn name(self) -> &'static str {
        match self {
            CodePage::Latin1 => "Latin-1",
            CodePage::Windows1252 => "Windows-1252",
            CodePage::Iso8859_15 => "ISO-8859-15",
            CodePage::Iso8859_2 => "ISO-8859-2",
            CodePage::Windows1250 => "Windows-1250",
            CodePage::Windows1251 => "Windows-1251",
            CodePage::Koi8R => "KOI8-R",
            CodePage::MacRoman => "Mac Roman",
            CodePage::Cp437 => "CP437",
            CodePage::Cp850 => "CP850",
            CodePage::Cp866 => "CP866",
        }
    }

    /// The page by the name it is shown under.
    pub fn by_name(name: &str) -> Option<CodePage> {
        CodePage::SLOT_A.iter().chain(CodePage::SLOT_B.iter()).copied().find(|p| p.name() == name)
    }

    /// The 128 characters above 0x7F. U+FFFD stands for a byte the page does
    /// not define, which is what makes the page refuse those bytes.
    pub fn high(self) -> &'static [char; 128] {
        match self {
            CodePage::Latin1 => &LATIN1_HIGH,
            CodePage::Windows1252 => &WINDOWS1252_HIGH,
            CodePage::Iso8859_15 => &ISO8859_15_HIGH,
            CodePage::Iso8859_2 => &ISO8859_2_HIGH,
            CodePage::Windows1250 => &WINDOWS1250_HIGH,
            CodePage::Windows1251 => &WINDOWS1251_HIGH,
            CodePage::Koi8R => &KOI8R_HIGH,
            CodePage::MacRoman => &MACROMAN_HIGH,
            CodePage::Cp437 => &CP437_HIGH,
            CodePage::Cp850 => &CP850_HIGH,
            CodePage::Cp866 => &CP866_HIGH,
        }
    }

    /// What this page reads a byte as, or U+FFFD where it defines nothing.
    pub fn char_of(self, b: u8) -> char {
        if b < 0x80 {
            b as char
        } else {
            self.high()[(b - 0x80) as usize]
        }
    }
}


/// The encoding once the vague cases have been settled by looking at the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Utf8,
    Ascii,
    /// One of the single-byte code pages, which one being the reader's choice.
    SingleByte(CodePage),
    Utf16(Endian),
}

impl Settled {
    /// The two pages that were here before the rest were, spelled the way the
    /// code that names them already does. Named after the variants they stand
    /// in for rather than shouted, since that is how they are read and written.
    #[allow(non_upper_case_globals)]
    pub const Latin1: Settled = Settled::SingleByte(CodePage::Latin1);
    #[allow(non_upper_case_globals)]
    pub const Cp437: Settled = Settled::SingleByte(CodePage::Cp437);

    /// Bytes per code unit: what a terminator, a pad and the scan step are made of.
    pub fn unit(self) -> usize {
        match self {
            Settled::Utf16(_) => 2,
            _ => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Settled::Utf8 => "UTF-8",
            Settled::Ascii => "ASCII",
            Settled::SingleByte(p) => p.name(),
            Settled::Utf16(Endian::Little) => "UTF-16 LE",
            Settled::Utf16(Endian::Big) => "UTF-16 BE",
        }
    }
}

/// What a field was read as, and how that was decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub text: String,
    pub settled: Settled,
    /// Bytes taken by a byte-order mark: part of the field, not of the value.
    pub bom: usize,
    /// True when the bytes do not fit the encoding and the text is a repair.
    pub lossy: bool,
    /// Set when the template did not name the encoding outright.
    pub note: Option<String>,
}

/// Decide the encoding from the first bytes without decoding them: what the
/// scanner needs before it can tell how long the field is.
pub fn settle(enc: &Encoding, head: &[u8]) -> (Settled, usize, Option<String>) {
    match enc {
        Encoding::Utf8 => (Settled::Utf8, 0, None),
        Encoding::Ascii => (Settled::Ascii, 0, None),
        Encoding::Latin1 => (Settled::Latin1, 0, None),
        Encoding::Cp437 => (Settled::Cp437, 0, None),
        Encoding::Utf16(e) => (Settled::Utf16(*e), 0, None),
        Encoding::Bom { fallback } => match head {
            [0xef, 0xbb, 0xbf, ..] => (Settled::Utf8, 3, Some("Read as UTF-8, from a byte-order mark".into())),
            [0xff, 0xfe, ..] => (Settled::Utf16(Endian::Little), 2, Some("Read as UTF-16 LE, from a byte-order mark".into())),
            [0xfe, 0xff, ..] => (Settled::Utf16(Endian::Big), 2, Some("Read as UTF-16 BE, from a byte-order mark".into())),
            _ => {
                let (s, _, _) = settle(fallback, head);
                (s, 0, Some(format!("Read as {}; no byte-order mark found", s.name())))
            }
        },
        // Not stated by the format: take UTF-8 if the bytes are valid UTF-8,
        // since arbitrary bytes rarely are, and Latin-1 otherwise.
        Encoding::Unknown => {
            if std::str::from_utf8(head).is_ok() {
                (Settled::Utf8, 0, Some("Read as UTF-8, a guess (valid UTF-8)".into()))
            } else {
                (Settled::Latin1, 0, Some("Read as Latin-1, a guess (not valid UTF-8)".into()))
            }
        }
    }
}

pub fn decode(enc: &Encoding, bytes: &[u8]) -> Reading {
    let (settled, bom, note) = settle(enc, bytes);
    let body = &bytes[bom.min(bytes.len())..];
    let (text, lossy) = decode_settled(settled, body);
    Reading { text, settled, bom, lossy, note }
}

pub fn decode_settled(settled: Settled, bytes: &[u8]) -> (String, bool) {
    match settled {
        Settled::Utf8 => match std::str::from_utf8(bytes) {
            Ok(s) => (s.to_string(), false),
            Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
        },
        Settled::Ascii => {
            let lossy = bytes.iter().any(|b| *b > 0x7f);
            let text = bytes
                .iter()
                .map(|b| if *b > 0x7f { char::REPLACEMENT_CHARACTER } else { *b as char })
                .collect();
            (text, lossy)
        }
        // A page that defines nothing for a byte cannot read it, and says so
        // the way the others do rather than passing a replacement off as text.
        Settled::SingleByte(p) => {
            let text: String = bytes.iter().map(|b| p.char_of(*b)).collect();
            let lossy = text.contains(char::REPLACEMENT_CHARACTER);
            (text, lossy)
        }
        Settled::Utf16(e) => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|p| match e {
                    Endian::Little => u16::from_le_bytes([p[0], p[1]]),
                    Endian::Big => u16::from_be_bytes([p[0], p[1]]),
                })
                .collect();
            // A trailing half unit means the field does not hold whole characters.
            let odd = bytes.len() % 2 != 0;
            match String::from_utf16(&units) {
                Ok(s) => (s, odd),
                Err(_) => (String::from_utf16_lossy(&units), true),
            }
        }
    }
}

pub fn cp437_char(b: u8) -> char {
    CodePage::Cp437.char_of(b)
}

/// Text to bytes in the encoding it was read as. A character the encoding
/// cannot hold is returned rather than quietly replaced.
pub fn encode_settled(settled: Settled, text: &str) -> Result<Vec<u8>, char> {
    match settled {
        Settled::Utf8 => Ok(text.as_bytes().to_vec()),
        Settled::Ascii => text.chars().map(|c| if c.is_ascii() { Ok(c as u8) } else { Err(c) }).collect(),
        // The table read backwards. A page's undefined slots are skipped, so
        // U+FFFD is a character no page can hold rather than a way in.
        Settled::SingleByte(p) => text
            .chars()
            .map(|c| match p.high().iter().position(|x| *x == c && *x != char::REPLACEMENT_CHARACTER) {
                Some(i) => Ok(0x80 + i as u8),
                None if c.is_ascii() => Ok(c as u8),
                None => Err(c),
            })
            .collect(),
        Settled::Utf16(e) => {
            let mut out = Vec::with_capacity(text.len() * 2);
            for u in text.encode_utf16() {
                out.extend_from_slice(&match e {
                    Endian::Little => u.to_le_bytes(),
                    Endian::Big => u.to_be_bytes(),
                });
            }
            Ok(out)
        }
    }
}

/// Bytes written the way C writes a string: the printable ones as they are,
/// the rest as escapes. A PNG's signature reads `"\x89PNG\r\n\x1a\n"`, which
/// says both that it starts with a byte no text file has and that the rest of
/// it is the word PNG.
///
/// The escapes are the ones C defines, and are kept unambiguous the way C
/// needs: `\x89` swallows every hex digit after it, so a byte that would be
/// read into the escape before it is written in octal instead, which is
/// always three digits and stops there.
///
/// That costs a signature two bases at once: Matroska reads `"\032E\xdf\xa3"`,
/// where `\032` and the `0x1a` in the gutter are the same byte written two
/// ways. Rust and Python stop `\x` after two digits and would write `\x1a`
/// there, and which language's rules to follow is worth offering as a setting
/// rather than deciding here. Until it is one, C's rules stand: a string that
/// is wrong in C is wrong without saying so, and this is the safe direction to
/// be wrong in.
pub fn c_string(bytes: &[u8]) -> String {
    let hex = |b: u8| b.is_ascii_hexdigit();
    let octal = |b: u8| b.is_ascii_digit() && b < b'8';
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('"');
    for (i, &b) in bytes.iter().enumerate() {
        let next = bytes.get(i + 1).copied();
        match b {
            b'"' => out.push_str(r#"\""#),
            b'\\' => out.push_str(r"\\"),
            0x20..=0x7e => out.push(b as char),
            0x07 => out.push_str(r"\a"),
            0x08 => out.push_str(r"\b"),
            0x09 => out.push_str(r"\t"),
            0x0a => out.push_str(r"\n"),
            0x0b => out.push_str(r"\v"),
            0x0c => out.push_str(r"\f"),
            0x0d => out.push_str(r"\r"),
            0 if !next.is_some_and(octal) => out.push_str(r"\0"),
            _ if next.is_some_and(hex) => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\{b:03o}"));
            }
            _ => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\x{b:02x}"));
            }
        }
    }
    out.push('"');
    out
}

/// A language to write the selected bytes into, as that language's own string
/// literal. C's rules are not everyone's: `\x` stops after two digits in most
/// of these and swallows digits in C, a byte string is spelled `b"…"` in two
/// of them, and JSON has no `\x` at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    C,
    Rust,
    Python,
    JavaScript,
    Json,
    CSharp,
    Go,
}

impl Lang {
    pub const ALL: [Lang; 7] =
        [Lang::C, Lang::Rust, Lang::Python, Lang::JavaScript, Lang::Json, Lang::CSharp, Lang::Go];

    pub fn name(self) -> &'static str {
        match self {
            Lang::C => "C",
            Lang::Rust => "Rust",
            Lang::Python => "Python",
            Lang::JavaScript => "JavaScript",
            Lang::Json => "JSON",
            Lang::CSharp => "C#",
            Lang::Go => "Go",
        }
    }

    pub fn by_name(name: &str) -> Option<Lang> {
        Lang::ALL.iter().copied().find(|l| l.name() == name)
    }
}

/// The escape every one of these languages spells the same way, or nothing.
fn named_escape(c: char) -> Option<&'static str> {
    Some(match c {
        '"' => "\\\"",
        '\\' => r"\\",
        '\n' => r"\n",
        '\r' => r"\r",
        '\t' => r"\t",
        _ => return None,
    })
}

/// What has to be escaped to survive being read back: the C0 controls, DEL and
/// the C1 range. Everything else above ASCII is a character and is written as
/// one, since a literal full of `é` says less than one holding `é`.
fn needs_escape(c: char) -> bool {
    let u = c as u32;
    u < 0x20 || u == 0x7f || (0x80..=0x9f).contains(&u)
}

/// Bytes as a string literal in `lang`, on one line: the printable ASCII as it
/// is, everything else escaped the way that language escapes it.
///
/// Where the bytes are valid UTF-8 the languages that have a text type get a
/// text literal, which is the one a reader can recognise words in; where they
/// are not, the ones that have a byte-string type get that instead, and the
/// ones that do not get one code unit per byte.
pub fn literal(lang: Lang, bytes: &[u8]) -> String {
    match lang {
        Lang::C => c_string(bytes),
        // Go strings are bytes, and `\x` there stops after two digits, so one
        // rule covers both valid and invalid UTF-8.
        Lang::Go => byte_literal(lang, bytes),
        _ => match std::str::from_utf8(bytes) {
            Ok(s) => text_literal(lang, s),
            Err(_) => byte_literal(lang, bytes),
        },
    }
}

/// One escape per byte, for bytes that are not text.
fn byte_literal(lang: Lang, bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 3);
    if matches!(lang, Lang::Rust | Lang::Python) {
        out.push('b');
    }
    out.push('"');
    for &b in bytes {
        match named_escape(b as char) {
            Some(e) => out.push_str(e),
            None if (0x20..=0x7e).contains(&b) => out.push(b as char),
            None => match lang {
                Lang::Json | Lang::CSharp => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\u{b:04x}"));
                }
                _ => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\x{b:02x}"));
                }
            },
        }
    }
    out.push('"');
    out
}

/// Text as that language writes text.
fn text_literal(lang: Lang, s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        let u = c as u32;
        // U+2028 and U+2029 end a line in JavaScript, and a literal is one line.
        let hidden = matches!(lang, Lang::JavaScript) && (u == 0x2028 || u == 0x2029);
        if let Some(e) = named_escape(c) {
            out.push_str(e);
        } else if !needs_escape(c) && !hidden {
            out.push(c);
        } else {
            match lang {
                Lang::Rust => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\u{{{u:x}}}"));
                }
                Lang::Python if u < 0x80 => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\x{u:02x}"));
                }
                Lang::Python if u <= 0xffff => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\u{u:04x}"));
                }
                Lang::Python => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\U{u:08x}"));
                }
                Lang::JavaScript if u < 0x80 => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\x{u:02x}"));
                }
                Lang::JavaScript => {
                    // A code unit at a time, so an astral character is the
                    // surrogate pair JavaScript stores it as.
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\u{unit:04x}"));
                    }
                }
                // C# has no `\x` worth using: it is variable-length there and
                // runs into the digit after it, exactly the trap C's has.
                _ => {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\u{u:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

/// The bytes a pad or terminator takes in this encoding: one byte, or a whole
/// code unit for UTF-16.
pub fn unit_bytes(settled: Settled, byte: u8) -> Vec<u8> {
    match settled {
        Settled::Utf16(Endian::Little) => vec![byte, 0],
        Settled::Utf16(Endian::Big) => vec![0, byte],
        _ => vec![byte],
    }
}

/// Every way a run of bytes reads as text.
///
/// A hex editor's reader picks out some bytes and wants to know what they say.
/// Answering with six rows, one per encoding, is answering with noise: most
/// runs are printable ASCII, and every encoding here agrees on that range, so
/// five of the six rows would be the same sentence. So the readings that agree
/// are gathered together and the encodings that agree on one are named beside
/// it, which is a fact worth having on its own: bytes that read the same
/// whatever you assume are bytes nobody can misread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readings {
    /// One entry per distinct reading, with the encodings giving it. In the
    /// order the encodings are tried, so the first entry is the first
    /// encoding that produced it.
    pub agreed: Vec<(Vec<Settled>, String)>,
    /// Encodings the bytes do not fit: a high byte where ASCII has none, half
    /// a code unit at the end of a UTF-16 run, a byte sequence UTF-8 does not
    /// allow. Named rather than shown, since what they produce is a row of
    /// replacement characters that says nothing.
    pub refused: Vec<Settled>,
}

/// The encodings a run of bytes is offered as, in the order they are tried.
/// The same six the text view offers, so a reading found here can be turned on
/// there.
/// Two of the six are the reader's own choice: one page from the ISO, Windows
/// and Mac family and one from the DOS family, since a run of high bytes is
/// usually being read against one of each rather than against eleven.
pub fn offered(page_a: CodePage, page_b: CodePage) -> [Settled; 6] {
    [
        Settled::Utf8,
        Settled::Ascii,
        Settled::SingleByte(page_a),
        Settled::SingleByte(page_b),
        Settled::Utf16(Endian::Little),
        Settled::Utf16(Endian::Big),
    ]
}

/// Read `bytes` every offered way, gathering the encodings that agree.
///
/// `first` puts one encoding at the front of the order, which is what the
/// reader is most likely reading the file in. It changes which encoding gets
/// named first on a shared row and nothing else.
pub fn readings(bytes: &[u8], first: Option<Settled>, page_a: CodePage, page_b: CodePage) -> Readings {
    let mut order: Vec<Settled> = first.into_iter().collect();
    order.extend(offered(page_a, page_b).iter().copied().filter(|s| Some(*s) != first));
    let mut agreed: Vec<(Vec<Settled>, String)> = Vec::new();
    let mut refused = Vec::new();
    for enc in order {
        let (text, lossy) = decode_settled(enc, bytes);
        if lossy {
            refused.push(enc);
            continue;
        }
        match agreed.iter_mut().find(|(_, t)| *t == text) {
            Some((who, _)) => who.push(enc),
            None => agreed.push((vec![enc], text)),
        }
    }
    Readings { agreed, refused }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_bytes_read_the_same_whatever_you_assume() {
        let r = readings(b"hello", None, CodePage::Latin1, CodePage::Cp437);
        assert_eq!(r.agreed.len(), 1, "one reading, four encodings agreeing on it");
        let (who, text) = &r.agreed[0];
        assert_eq!(text, "hello");
        assert_eq!(who, &[Settled::Utf8, Settled::Ascii, Settled::Latin1, Settled::Cp437]);
        // Five bytes is not a whole number of UTF-16 code units.
        assert_eq!(r.refused, vec![Settled::Utf16(Endian::Little), Settled::Utf16(Endian::Big)]);
    }

    #[test]
    fn a_high_byte_is_where_the_encodings_part() {
        let r = readings(&[0xb0, 0xb1], None, CodePage::Latin1, CodePage::Cp437);
        let texts: Vec<&str> = r.agreed.iter().map(|(_, t)| t.as_str()).collect();
        assert!(texts.contains(&"\u{b0}\u{b1}"), "Latin-1 reads them as its own characters");
        assert!(texts.contains(&"\u{2591}\u{2592}"), "CP437 reads them as shading");
        assert!(r.refused.contains(&Settled::Ascii), "ASCII has no room for either");
        assert!(r.refused.contains(&Settled::Utf8), "and they are not valid UTF-8");
    }

    #[test]
    fn the_encoding_asked_for_first_is_named_first() {
        let r = readings(b"hi", Some(Settled::Cp437), CodePage::Latin1, CodePage::Cp437);
        assert_eq!(r.agreed[0].0[0], Settled::Cp437);
    }

    #[test]
    fn bytes_read_as_c_writes_them() {
        assert_eq!(c_string(b"GGUF"), r#""GGUF""#);
        assert_eq!(c_string(b"\x89PNG\r\n\x1a\n"), r#""\x89PNG\r\n\x1a\n""#);
        assert_eq!(c_string(b"\0asm"), r#""\0asm""#);
        assert_eq!(c_string(b"say \"hi\"\\"), r#""say \"hi\"\\""#);
        // A byte C would read into the escape before it goes in octal, which
        // ends after three digits: `\x1f` then `e` would be one escape.
        assert_eq!(c_string(&[0x1f, b'e']), r#""\037e""#);
        assert_eq!(c_string(&[0, b'7']), r#""\0007""#);
        assert_eq!(c_string(&[0, b'x']), r#""\0x""#);
    }

    #[test]
    fn cp437_landmarks_and_round_trip() {
        // Checked against Python's codec, which generated the table.
        assert_eq!(cp437_char(0x80), '\u{00c7}');
        assert_eq!(cp437_char(0xe1), '\u{00df}');
        assert_eq!(cp437_char(0xfd), '\u{00b2}');
        let all: Vec<u8> = (0u8..=255).collect();
        let (text, lossy) = decode_settled(Settled::Cp437, &all);
        assert!(!lossy);
        assert_eq!(encode_settled(Settled::Cp437, &text).unwrap(), all);
    }

    #[test]
    fn latin1_and_ascii() {
        let bytes = [0x41, 0xe9, 0xff];
        assert_eq!(decode_settled(Settled::Latin1, &bytes).0, "A\u{00e9}\u{00ff}");
        assert_eq!(encode_settled(Settled::Latin1, "A\u{00e9}\u{00ff}").unwrap(), bytes);
        assert_eq!(encode_settled(Settled::Latin1, "\u{20ac}"), Err('\u{20ac}'));
        let (text, lossy) = decode_settled(Settled::Ascii, &bytes);
        assert!(lossy);
        assert_eq!(text.chars().next(), Some('A'));
        assert_eq!(encode_settled(Settled::Ascii, "\u{00e9}"), Err('\u{00e9}'));
    }

    #[test]
    fn every_code_page_round_trips_the_bytes_it_defines() {
        // Checked against Python's codecs, which generated the tables.
        for page in CodePage::SLOT_A.iter().chain(CodePage::SLOT_B.iter()).copied() {
            let defined: Vec<u8> =
                (0u8..=255).filter(|b| page.char_of(*b) != char::REPLACEMENT_CHARACTER).collect();
            let (text, lossy) = decode_settled(Settled::SingleByte(page), &defined);
            assert!(!lossy, "{} refuses a byte it defines", page.name());
            assert_eq!(encode_settled(Settled::SingleByte(page), &text).unwrap(), defined, "{}", page.name());
            assert_eq!(CodePage::by_name(page.name()), Some(page));
        }
    }

    #[test]
    fn a_page_refuses_the_bytes_it_leaves_undefined() {
        // 0x81 is one of Windows-1252's five holes; Latin-1 has none.
        let (_, lossy) = decode_settled(Settled::SingleByte(CodePage::Windows1252), &[0x81]);
        assert!(lossy);
        assert!(!decode_settled(Settled::Latin1, &[0x81]).1);
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Windows1252), &[0x80]).0, "\u{20ac}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Koi8R), &[0xc1]).0, "\u{0430}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Cp866), &[0x80]).0, "\u{0410}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::MacRoman), &[0xa5]).0, "\u{2022}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Cp850), &[0xd5]).0, "\u{0131}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Iso8859_15), &[0xa4]).0, "\u{20ac}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Iso8859_2), &[0xb1]).0, "\u{0105}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Windows1250), &[0xe1]).0, "\u{00e1}");
        assert_eq!(decode_settled(Settled::SingleByte(CodePage::Windows1251), &[0xe0]).0, "\u{0430}");
        // A replacement character is not a way into a page's undefined slot.
        assert_eq!(encode_settled(Settled::SingleByte(CodePage::Windows1252), "\u{fffd}"), Err('\u{fffd}'));
    }

    #[test]
    fn the_offered_encodings_follow_the_chosen_pages() {
        let r = readings(&[0x80], None, CodePage::Windows1252, CodePage::Cp437);
        let texts: Vec<&str> = r.agreed.iter().map(|(_, t)| t.as_str()).collect();
        assert!(texts.contains(&"\u{20ac}"), "Windows-1252 reads 0x80 as a euro sign");
        assert!(!r.refused.contains(&Settled::SingleByte(CodePage::Windows1252)));
        // The page not chosen is not offered at all.
        let named: Vec<&str> =
            r.agreed.iter().flat_map(|(w, _)| w.iter()).chain(r.refused.iter()).map(|s| s.name()).collect();
        assert!(!named.contains(&"Latin-1"));
    }

    #[test]
    fn bytes_written_as_each_language_writes_them() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(literal(Lang::C, png), r#""\x89PNG\r\n\x1a\n""#);
        assert_eq!(literal(Lang::Go, png), r#""\x89PNG\r\n\x1a\n""#);
        assert_eq!(literal(Lang::Rust, png), r#"b"\x89PNG\r\n\x1a\n""#);
        assert_eq!(literal(Lang::Python, png), r#"b"\x89PNG\r\n\x1a\n""#);
        assert_eq!(literal(Lang::JavaScript, png), r#""\x89PNG\r\n\x1a\n""#);
        assert_eq!(literal(Lang::Json, png), r#""\u0089PNG\r\n\u001a\n""#);
        assert_eq!(literal(Lang::CSharp, png), r#""\u0089PNG\r\n\u001a\n""#);

        // Valid UTF-8: the languages with a text type say the words.
        let caf = "caf\u{e9}".as_bytes();
        assert_eq!(literal(Lang::Rust, caf), "\"caf\u{e9}\"");
        assert_eq!(literal(Lang::Python, caf), "\"caf\u{e9}\"");
        assert_eq!(literal(Lang::JavaScript, caf), "\"caf\u{e9}\"");
        assert_eq!(literal(Lang::Json, caf), "\"caf\u{e9}\"");
        assert_eq!(literal(Lang::CSharp, caf), "\"caf\u{e9}\"");
        // Go and C are bytes whatever they hold.
        assert_eq!(literal(Lang::Go, caf), r#""caf\xc3\xa9""#);

        // Quotes and backslashes go the same way everywhere.
        for lang in Lang::ALL {
            assert_eq!(literal(lang, br#"say "hi"\"#), r#""say \"hi\"\\""#, "{}", lang.name());
        }

        // A control inside text, escaped the way each language escapes it.
        let bell = "a\u{7}b".as_bytes();
        assert_eq!(literal(Lang::Rust, bell), r#""a\u{7}b""#);
        assert_eq!(literal(Lang::Python, bell), r#""a\x07b""#);
        assert_eq!(literal(Lang::JavaScript, bell), r#""a\x07b""#);
        assert_eq!(literal(Lang::Json, bell), r#""a\u0007b""#);
        assert_eq!(literal(Lang::CSharp, bell), r#""a\u0007b""#);

        // Astral characters: one escape in Python, a surrogate pair in JS,
        // and the character itself where it is printable.
        let emoji = "\u{1f600}".as_bytes();
        assert_eq!(literal(Lang::Python, emoji), "\"\u{1f600}\"");
        assert_eq!(literal(Lang::JavaScript, emoji), "\"\u{1f600}\"");
        // A JavaScript line terminator is not printable in a one-line literal.
        assert_eq!(literal(Lang::JavaScript, "\u{2028}".as_bytes()), r#""\u2028""#);
        assert_eq!(literal(Lang::Json, "\u{2028}".as_bytes()), "\"\u{2028}\"");
        // C1 is escaped even though it is above ASCII.
        assert_eq!(literal(Lang::Python, "\u{85}".as_bytes()), r#""\u0085""#);
        assert_eq!(literal(Lang::Rust, "\u{85}".as_bytes()), r#""\u{85}""#);
        assert_eq!(literal(Lang::Python, "\u{10ffff}".as_bytes()), "\"\u{10ffff}\"");

        assert_eq!(Lang::by_name("C#"), Some(Lang::CSharp));
        assert_eq!(Lang::by_name("Objective-C"), None);
    }

    #[test]
    fn utf16_both_ways() {
        let le = encode_settled(Settled::Utf16(Endian::Little), "Hi").unwrap();
        assert_eq!(le, vec![0x48, 0, 0x69, 0]);
        assert_eq!(decode_settled(Settled::Utf16(Endian::Little), &le).0, "Hi");
        let be = encode_settled(Settled::Utf16(Endian::Big), "Hi").unwrap();
        assert_eq!(be, vec![0, 0x48, 0, 0x69]);
        // Half a code unit at the end is a broken field, and says so.
        assert!(decode_settled(Settled::Utf16(Endian::Big), &[0, 0x48, 0]).1);
        assert_eq!(unit_bytes(Settled::Utf16(Endian::Little), 0), vec![0, 0]);
    }

    #[test]
    fn boms_and_guesses() {
        let bom = Encoding::Bom { fallback: Box::new(Encoding::Latin1) };
        let r = decode(&bom, &[0xff, 0xfe, 0x48, 0x00]);
        assert_eq!(r.settled, Settled::Utf16(Endian::Little));
        assert_eq!(r.bom, 2);
        assert_eq!(r.text, "H");
        assert_eq!(r.note.as_deref(), Some("Read as UTF-16 LE, from a byte-order mark"));

        let plain = decode(&bom, &[0x48, 0xe9]);
        assert_eq!(plain.settled, Settled::Latin1);
        assert_eq!(plain.bom, 0);
        assert_eq!(plain.text, "H\u{00e9}");

        let utf8 = decode(&Encoding::Unknown, "caf\u{00e9}".as_bytes());
        assert_eq!(utf8.settled, Settled::Utf8);
        assert_eq!(utf8.text, "caf\u{00e9}");
        let latin = decode(&Encoding::Unknown, &[0x63, 0x61, 0x66, 0xe9]);
        assert_eq!(latin.settled, Settled::Latin1);
        assert_eq!(latin.text, "caf\u{00e9}");
    }
}
