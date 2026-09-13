# Bundled Kaitai Struct formats

Verbatim copies of `.ksy` files from the [Kaitai Struct format
library](https://github.com/kaitai-io/kaitai_struct_formats), compiled into
`qubero-core` and converted on demand. `crates/core/src/ksy/bundled.rs` is the
table; this file is the record of which formats are here and why.

Both are generated. `node tools/ksy_bundle.mjs` rewrites them from the
directory, so adding a format is: copy the `.ksy` in, run the script, run the
tests. Nothing here is edited by hand except the reasons, which live in the
script.

Each file keeps its own licence, which is in its `meta/license` and again in
`THIRD-PARTY-NOTICES.md` beside the licence text and the URL it came from. The
only licences that may appear are `CC0-1.0`, `MIT`, `Unlicense`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`.

## What the columns say

* **Bundled** is `yes` for a format a reader can open a file as, and `import`
  for a file that is here because another one imports it, or whose root type
  takes parameters nobody can supply.
* **Gaps** is how many things in the `.ksy` the IR cannot say. The conversion
  report names every one of them, and the panel shows them. A test asserts this
  number, so the table cannot drift from the code.
* **Sniffs** is how much evidence a dropped file has to match for this format to
  be offered: the first `seq` field's `contents`, plus any further `contents`
  the walk reaches through fields of a fixed width. Built-in formats are asked
  first, always, so this only ever answers for a file no builtin claims.

## Bundled

| Format | Category | Licence | Bundled | Gaps | Sniffs | Why |
| --- | --- | --- | --- | --- | --- | --- |
| `allegro_dat` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `amlogic_emmc_partitions` | filesystem | CC0-1.0 | yes | 0 | 4 bytes | converts whole |
| `android_bootldr_asus` | archive | CC0-1.0 | yes | 1 | no, same magic as android_bootldr_qcom | one instance naming a file inside the image is text, and the IR's expressions are integers |
| `android_bootldr_huawei` | archive | CC0-1.0 | yes | 0 | no magic | converts whole |
| `android_bootldr_qcom` | archive | CC0-1.0 | yes | 0 | no, same magic as android_bootldr_asus | converts whole |
| `android_dto` | archive | CC0-1.0 | yes | 0 | no magic | converts whole |
| `android_img` | archive | CC0-1.0 | yes | 0 | 8 bytes | converts whole |
| `android_nanoapp_header` | executable | Apache-2.0 | yes | 0 | no magic | converts whole |
| `android_super` | filesystem | CC0-1.0 | yes | 3 | no magic | three bit fields, all of them reserved padding, are read most-significant-bit first instead of least |
| `apm_partition_table` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `avantes_roh60` | scientific | CC0-1.0 | yes | 0 | no magic | converts whole |
| `avi` | media | CC0-1.0 | yes | 0 | 8 bytes | converts whole |
| `bcd` | common | CC0-1.0 | import | 0 | no magic | a root type with 3 parameters: nothing can open a file as it, and it is here to be imported |
| `bitcoin_transaction` | network | MIT | yes | 0 | no magic | converts whole |
| `btrfs_stream` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `bytes_with_io` | common | MIT | import | 0 | no magic | imported by six formats that need a sub-stream; on its own it says the file is bytes, which is what no template says already |
| `chrome_pak` | serialization | CC0-1.0 | yes | 0 | no magic | converts whole |
| `compressed_resource` | macos | MIT | yes | 1 | no magic | one instance reads inside another field's stream; the header and the compressed run are placed |
| `cramfs` | filesystem | MIT | yes | 0 | no magic | converts whole |
| `creative_voice_file` | media | CC0-1.0 | yes | 3 | 20 bytes | three sample-rate instances are floating point; every block is placed |
| `dcmp_0` | macos | MIT | yes | 2 | no magic | two instances of the decompressor's own bookkeeping; the compressed run is placed |
| `dcmp_1` | macos | MIT | yes | 2 | no magic | two instances of the decompressor's own bookkeeping; the compressed run is placed |
| `dcmp_2` | macos | MIT | import | 2 | no magic | a root type with 2 parameters: nothing can open a file as it, and it is here to be imported |
| `dcmp_variable_length_integer` | macos | MIT | yes | 1 | no magic | the one instance that reassembles the integer needs a bitwise or |
| `dex` | executable | Apache-2.0 | yes | 0 | no magic | converts whole |
| `dicom` | image | MIT | yes | 3 | no magic | two tag numbers and one transfer-syntax test are instances; every data element is placed |
| `dime_message` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `dns_packet` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `dos_datetime` | common | CC0-1.0 | yes | 8 | no magic | the packed date and time read most-significant-bit first instead of least, and six instances only zero-pad the parts for display |
| `ds_store` | macos | MIT | yes | 3 | no magic | three instances of the B-tree's bookkeeping; every record is placed |
| `dune_2_pak` | game | CC0-1.0 | yes | 1 | no magic | one instance measures the last entry against the length of the file, which the IR has no expression for |
| `edid` | hardware | CC0-1.0 | yes | 18 | 8 bytes | every gap is an instance: nine are floating-point colour coordinates, eight reassemble a ten-bit number, one spells the manufacturer |
| `efivar_signature_list` | security | CC0-1.0 | yes | 0 | no magic | converts whole |
| `ethernet_frame` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `ext2` | filesystem | CC0-1.0 | yes | 3 | no magic | three instances reaching into the first block group; the superblock and the group descriptors are placed |
| `fallout_dat` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `fallout2_dat` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `ftl_dat` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `genmidi_op2` | media | CC0-1.0 | yes | 0 | 8 bytes | converts whole |
| `gimp_brush` | image | CC0-1.0 | yes | 0 | no magic | converts whole |
| `google_protobuf` | serialization | MIT | yes | 0 | no magic | converts whole |
| `gpt_partition_table` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `gran_turismo_vol` | game | CC0-1.0 | yes | 0 | 12 bytes | converts whole |
| `grub2_font` | font | CC0-1.0 | yes | 0 | 12 bytes | converts whole |
| `hashcat_restore` | log | CC0-1.0 | yes | 0 | no magic | converts whole |
| `hccap` | network | Unlicense | yes | 1 | no magic | one instance re-reads the EAPOL buffer as its own stream |
| `hccapx` | network | Unlicense | yes | 0 | no magic | converts whole |
| `heaps_pak` | game | MIT | yes | 0 | no magic | converts whole |
| `heroes_of_might_and_magic_agg` | game | CC0-1.0 | yes | 1 | no magic | one instance measures a field the IR does not let it measure; the entry table is placed |
| `heroes_of_might_and_magic_bmp` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `icc_4` | image | CC0-1.0 | yes | 4 | no magic | the 8-bit and 16-bit LUT tag types need an exclusive or for the size of their table, which costs those two tags' last two fields; the header and the tag table are placed |
| `icmp_packet` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `ipv4_packet` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `ipv6_packet` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `java_class` | executable | CC0-1.0 | yes | 0 | 4 bytes | converts whole |
| `luks` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `mac_os_resource_snd` | macos | MIT | yes | 2 | no magic | two sample-rate instances are floating point |
| `magicavoxel_vox` | media | MIT | yes | 0 | 4 bytes | converts whole |
| `mbr_partition_table` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `mcap` | log | Apache-2.0 | yes | 2 | no magic | two instances, one measuring the file and one reading a value worked out later; every record is placed |
| `microsoft_network_monitor_v2` | network | CC0-1.0 | yes | 0 | 4 bytes | converts whole |
| `mifare_classic` | hardware | BSD-2-Clause | yes | 7 | no magic | seven instances: access-condition arithmetic and the value-block checks, all of which need bitwise operators |
| `minecraft_nbt` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `mozilla_mar` | archive | CC0-1.0 | yes | 0 | 4 bytes | converts whole |
| `nt_mdt_pal` | scientific | Unlicense | yes | 0 | 26 bytes | converts whole |
| `openpgp_message` | security | MIT | yes | 0 | no magic | converts whole |
| `pcf_font` | font | CC0-1.0 | yes | 3 | 4 bytes | three instances reading the string table as its own stream; every table is placed |
| `pcx` | image | CC0-1.0 | import | 0 | no magic | imported by pcx_dcx; Qubero reads a PCX with its own template |
| `pcx_dcx` | image | CC0-1.0 | yes | 0 | 4 bytes | converts whole |
| `phar_without_stub` | archive | CC0-1.0 | yes | 4 | no magic | four instances read a decimal count out of text; the manifest and every entry are placed |
| `php_serialized_value` | serialization | CC0-1.0 | import | 6 | no magic | imported by phar_without_stub; on its own, a mapping entry is another php_serialized_value, which the converter cannot say |
| `protocol_body` | network | CC0-1.0 | import | 0 | no magic | a root type with 1 parameter: nothing can open a file as it, and it is here to be imported |
| `psx_tim` | image | CC0-1.0 | yes | 0 | 4 bytes | converts whole |
| `python_pyc_27` | executable | CC0-1.0 | yes | 0 | no magic | converts whole |
| `regf` | windows | CC0-1.0 | yes | 0 | no magic | converts whole |
| `resource_fork` | macos | MIT | yes | 3 | no magic | three instances reading the name table and the data blocks as their own streams; the map and the type list are placed |
| `riff` | common | CC0-1.0 | yes | 6 | no magic | six instances re-reading a chunk's data as its own stream, and one spelling the chunk id; every chunk is placed |
| `rtp_packet` | network | Unlicense | import | 0 | no magic | imported by rtpdump; a packet on the wire, with no file form of its own |
| `rtpdump` | network | Unlicense | yes | 0 | no magic | converts whole |
| `ruby_marshal` | serialization | CC0-1.0 | yes | 1 | 2 bytes | the one instance that unpacks a small integer needs a bitwise complement |
| `saints_row_2_vpp_pc` | game | MIT | yes | 2 | 5 bytes | two instances reading the name tables as their own streams; the entry table is placed |
| `shapefile_index` | geospatial | CC0-1.0 | yes | 0 | no magic | converts whole |
| `shapefile_main` | geospatial | CC0-1.0 | yes | 0 | no magic | converts whole |
| `specpr` | scientific | Unlicense | yes | 2 | no magic | two floating-point instances |
| `ssh_public_key` | security | CC0-1.0 | yes | 0 | no magic | converts whole |
| `sudoers_ts` | log | CC0-1.0 | yes | 0 | no magic | converts whole |
| `tcp_segment` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `tls_client_hello` | network | MIT | yes | 0 | no magic | converts whole |
| `tr_dos_image` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `tsm` | database | MIT | yes | 0 | no magic | converts whole |
| `ttf` | font | MIT | yes | 3 | no magic | three instances reading a sub-table through its parent's stream; every table is placed |
| `udp_datagram` | network | CC0-1.0 | yes | 0 | no magic | converts whole |
| `uefi_te` | executable | CC0-1.0 | yes | 1 | no magic | one instance measures a field the IR does not let it measure |
| `uimage` | firmware | CC0-1.0 | yes | 0 | no magic | converts whole |
| `utf8_string` | common | CC0-1.0 | yes | 1 | no magic | the one instance that assembles a code point needs a bitwise or |
| `vfat` | filesystem | CC0-1.0 | yes | 8 | no magic | the packed date and time read most-significant-bit first instead of least, and six instances only zero-pad the parts for display |
| `vlq_base128_be` | common | CC0-1.0 | yes | 0 | no magic | converts whole |
| `vlq_base128_le` | common | CC0-1.0 | yes | 0 | no magic | converts whole |
| `vmware_vmdk` | filesystem | CC0-1.0 | yes | 0 | no magic | converts whole |
| `vp8_duck_ivf` | media | CC0-1.0 | yes | 0 | 8 bytes | converts whole |
| `warcraft_2_pud` | game | CC0-1.0 | yes | 0 | no magic | converts whole |
| `windows_evt_log` | log | CC0-1.0 | yes | 0 | no magic | converts whole |
| `windows_minidump` | windows | CC0-1.0 | yes | 0 | 6 bytes | converts whole |
| `windows_shell_items` | windows | CC0-1.0 | yes | 0 | no magic | converts whole |
| `windows_systemtime` | windows | CC0-1.0 | yes | 0 | no magic | converts whole |
| `wmf` | image | CC0-1.0 | yes | 0 | no magic | converts whole |
| `xwd` | image | CC0-1.0 | yes | 0 | no magic | converts whole |
| `zisofs` | archive | CC0-1.0 | yes | 1 | no magic | one instance with no `pos`, which has no place in the file the IR can name |

## Not bundled: the conversion does not reach the file

Candidates from `HANDOVER-kaitai-gaplist.md` whose report has gaps on the path
through the file: the root's own `seq`, or the type most of the file is made
of. A format that reads the front of a file and then loses the rest is worse
than no format at all, so these wait for the converter to grow.

| Format | Category | Licence | Bundled | Gaps | Sniffs | Why |
| --- | --- | --- | --- | --- | --- | --- |
| `android_sparse` | | CC0-1.0 | no | | | the size of a chunk's header reads `_root.header`, which a field of the same name hides, so no chunk body is placed |
| `asn1_der` | | CC0-1.0 | no | | | a sequence's body is another `asn1_der`, and the converter does not register a root type under its own name |
| `bson` | | CC0-1.0 | no | | | an element's value is another `bson` document, and the converter does not register a root type under its own name |
| `dbf` | | CC0-1.0 | no | | | a record's field widths come from `.length` over the header's field list, which the IR cannot take |
| `gettext_mo` | | BSD-2-Clause | no | | | the whole file's endianness is chosen from its first word, and the IR has no form for that |
| `msgpack` | | CC0-1.0 | no | | | the root holds another `msgpack`, and the converter does not register a root type under its own name |
| `nitf` | | MIT | no | | | every sub-header's count is text read as a number, so twenty-one fields on the path through the file are left as bytes |
| `packet_ppi` | | CC0-1.0 | no | | | the packet body switches to `packet_ppi` for PPI inside PPI, and the converter does not register a root type under its own name |
| `pcap` | | CC0-1.0 | no | | | the whole file's endianness is chosen from its magic, and the IR has no form for that |
| `windows_resource_file` | | CC0-1.0 | no | | | a resource's name is a `repeat-until` over `_`, which the IR cannot say, so everything after the name misplaces |

## Not bundled: the licence

From the gap list. These are not copied into this repository at all.

| Format | Licence |
| --- | --- |
| `vdi` | GPL-3.0 |
| `nt_mdt` | GPL-3.0 |
| `broadcom_trx` | GPL-2.0 |
| `lvm2` | GFDL-1.3 |
| `pif` | LGPL-2.1 |
| `renderware_binary_stream` | no licence tag |

## Notes

* 19 of the 108 files carry no `meta/title`. Nothing invents one: the
  chooser shows those by their id.
* 22 formats declare a magic and 20 of them sniff. None was dropped for
  having a one-byte magic, because none has one; 2 were dropped for sharing
  a magic with each other, which the table names.
* `ruby_marshal` (`04 08`) and `psx_tim` (`10 00 00 00`) pass the two-byte rule
  with weak evidence: both are version numbers rather than a name. They sniff
  only for a file no builtin claims, which is the whole reason that is
  tolerable.
