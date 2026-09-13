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
* **Reason** is filled in only where a row needs explaining: why a format with
  gaps ships anyway, or why a file is here to be imported and not offered. A
  blank means the format converts whole and a reader can pick it.

## Bundled

| Format | Category | Licence | Bundled | Gaps | Sniffs | Reason |
| --- | --- | --- | --- | --- | --- | --- |
| `allegro_dat` | game | CC0-1.0 | yes | 0 | no magic |  |
| `amlogic_emmc_partitions` | filesystem | CC0-1.0 | yes | 0 | 4 bytes |  |
| `android_bootldr_asus` | archive | CC0-1.0 | yes | 1 | no, same magic as android_bootldr_qcom | one instance naming a file inside the image is text, and the IR's expressions are integers |
| `android_bootldr_huawei` | archive | CC0-1.0 | yes | 0 | no magic |  |
| `android_bootldr_qcom` | archive | CC0-1.0 | yes | 0 | no, same magic as android_bootldr_asus |  |
| `android_dto` | archive | CC0-1.0 | yes | 0 | no magic |  |
| `android_img` | archive | CC0-1.0 | yes | 0 | 8 bytes |  |
| `android_nanoapp_header` | executable | Apache-2.0 | yes | 0 | no magic |  |
| `android_super` | filesystem | CC0-1.0 | yes | 3 | no magic | three bit fields, all of them reserved padding, are read most-significant-bit first instead of least |
| `apm_partition_table` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `asn1_der` | serialization | CC0-1.0 | yes | 0 | no magic |  |
| `avantes_roh60` | scientific | CC0-1.0 | yes | 0 | no magic |  |
| `avi` | media | CC0-1.0 | yes | 0 | 8 bytes |  |
| `bcd` | common | CC0-1.0 | import | 0 | no magic | a root type with 3 parameters: nothing can open a file as it, and it is here to be imported |
| `bitcoin_transaction` | network | MIT | yes | 0 | no magic |  |
| `bson` | serialization | CC0-1.0 | yes | 1 | no magic | the one instance that unpacks a three-byte integer needs a bitwise or; every element is placed |
| `btrfs_stream` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `bytes_with_io` | common | MIT | import | 0 | no magic | imported by six formats that need a sub-stream; on its own it says the file is bytes, which is what no template says already |
| `chrome_pak` | serialization | CC0-1.0 | yes | 0 | no magic |  |
| `compressed_resource` | macos | MIT | yes | 1 | no magic | one instance reads inside another field's stream; the header and the compressed run are placed |
| `cramfs` | filesystem | MIT | yes | 0 | no magic |  |
| `creative_voice_file` | media | CC0-1.0 | yes | 3 | 20 bytes | three sample-rate instances are floating point; every block is placed |
| `dcmp_0` | macos | MIT | yes | 2 | no magic | two instances of the decompressor's own bookkeeping; the compressed run is placed |
| `dcmp_1` | macos | MIT | yes | 2 | no magic | two instances of the decompressor's own bookkeeping; the compressed run is placed |
| `dcmp_2` | macos | MIT | import | 2 | no magic | a root type with 2 parameters: nothing can open a file as it, and it is here to be imported |
| `dcmp_variable_length_integer` | macos | MIT | yes | 1 | no magic | the one instance that reassembles the integer needs a bitwise or |
| `dex` | executable | Apache-2.0 | yes | 0 | no magic |  |
| `dicom` | image | MIT | yes | 3 | no magic | two tag numbers and one transfer-syntax test are instances; every data element is placed |
| `dime_message` | network | CC0-1.0 | yes | 0 | no magic |  |
| `dns_packet` | network | CC0-1.0 | yes | 0 | no magic |  |
| `dos_datetime` | common | CC0-1.0 | yes | 8 | no magic | the packed date and time read most-significant-bit first instead of least, and six instances only zero-pad the parts for display |
| `ds_store` | macos | MIT | yes | 3 | no magic | three instances of the B-tree's bookkeeping; every record is placed |
| `dune_2_pak` | game | CC0-1.0 | yes | 1 | no magic | one instance measures the last entry against the length of the file, which the IR has no expression for |
| `edid` | hardware | CC0-1.0 | yes | 18 | 8 bytes | every gap is an instance: nine are floating-point colour coordinates, eight reassemble a ten-bit number, one spells the manufacturer |
| `efivar_signature_list` | security | CC0-1.0 | yes | 0 | no magic |  |
| `ethernet_frame` | network | CC0-1.0 | yes | 0 | no magic |  |
| `ext2` | filesystem | CC0-1.0 | yes | 3 | no magic | three instances reaching into the first block group; the superblock and the group descriptors are placed |
| `fallout_dat` | game | CC0-1.0 | yes | 0 | no magic |  |
| `fallout2_dat` | game | CC0-1.0 | yes | 0 | no magic |  |
| `ftl_dat` | game | CC0-1.0 | yes | 0 | no magic |  |
| `genmidi_op2` | media | CC0-1.0 | yes | 0 | 8 bytes |  |
| `gimp_brush` | image | CC0-1.0 | yes | 0 | no magic |  |
| `google_protobuf` | serialization | MIT | yes | 0 | no magic |  |
| `gpt_partition_table` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `gran_turismo_vol` | game | CC0-1.0 | yes | 0 | 12 bytes |  |
| `grub2_font` | font | CC0-1.0 | yes | 0 | 12 bytes |  |
| `hashcat_restore` | log | CC0-1.0 | yes | 0 | no magic |  |
| `hccap` | network | Unlicense | yes | 1 | no magic | one instance re-reads the EAPOL buffer as its own stream |
| `hccapx` | network | Unlicense | yes | 0 | no magic |  |
| `heaps_pak` | game | MIT | yes | 0 | no magic |  |
| `heroes_of_might_and_magic_agg` | game | CC0-1.0 | yes | 1 | no magic | one instance measures a field the IR does not let it measure; the entry table is placed |
| `heroes_of_might_and_magic_bmp` | game | CC0-1.0 | yes | 0 | no magic |  |
| `icc_4` | image | CC0-1.0 | yes | 4 | no magic | the 8-bit and 16-bit LUT tag types need an exclusive or for the size of their table, which costs those two tags' last two fields; the header and the tag table are placed |
| `icmp_packet` | network | CC0-1.0 | yes | 0 | no magic |  |
| `ipv4_packet` | network | CC0-1.0 | yes | 0 | no magic |  |
| `ipv6_packet` | network | CC0-1.0 | yes | 0 | no magic |  |
| `java_class` | executable | CC0-1.0 | yes | 0 | 4 bytes |  |
| `luks` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `mac_os_resource_snd` | macos | MIT | yes | 2 | no magic | two sample-rate instances are floating point |
| `magicavoxel_vox` | media | MIT | yes | 0 | 4 bytes |  |
| `mbr_partition_table` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `mcap` | log | Apache-2.0 | yes | 2 | no magic | two instances, one measuring the file and one reading a value worked out later; every record is placed |
| `microsoft_network_monitor_v2` | network | CC0-1.0 | yes | 0 | 4 bytes |  |
| `mifare_classic` | hardware | BSD-2-Clause | yes | 7 | no magic | seven instances: access-condition arithmetic and the value-block checks, all of which need bitwise operators |
| `minecraft_nbt` | game | CC0-1.0 | yes | 0 | no magic |  |
| `mozilla_mar` | archive | CC0-1.0 | yes | 0 | 4 bytes |  |
| `msgpack` | serialization | CC0-1.0 | yes | 0 | no magic |  |
| `nt_mdt_pal` | scientific | Unlicense | yes | 0 | 26 bytes |  |
| `openpgp_message` | security | MIT | yes | 0 | no magic |  |
| `packet_ppi` | network | CC0-1.0 | yes | 0 | no magic |  |
| `pcf_font` | font | CC0-1.0 | yes | 3 | 4 bytes | three instances reading the string table as its own stream; every table is placed |
| `pcx` | image | CC0-1.0 | import | 0 | no magic | imported by pcx_dcx; Qubero reads a PCX with its own template |
| `pcx_dcx` | image | CC0-1.0 | yes | 0 | 4 bytes |  |
| `phar_without_stub` | archive | CC0-1.0 | yes | 4 | no magic | four instances read a decimal count out of text; the manifest and every entry are placed |
| `php_serialized_value` | serialization | CC0-1.0 | import | 4 | no magic | imported by phar_without_stub; on its own, every string's length is a decimal count read out of text, which a template does only where the field is declared as digits |
| `protocol_body` | network | CC0-1.0 | import | 0 | no magic | a root type with 1 parameter: nothing can open a file as it, and it is here to be imported |
| `psx_tim` | image | CC0-1.0 | yes | 0 | 4 bytes |  |
| `python_pyc_27` | executable | CC0-1.0 | yes | 0 | no magic |  |
| `regf` | windows | CC0-1.0 | yes | 0 | no magic |  |
| `resource_fork` | macos | MIT | yes | 3 | no magic | three instances reading the name table and the data blocks as their own streams; the map and the type list are placed |
| `riff` | common | CC0-1.0 | yes | 6 | no magic | six instances re-reading a chunk's data as its own stream, and one spelling the chunk id; every chunk is placed |
| `rtp_packet` | network | Unlicense | import | 0 | no magic | imported by rtpdump; a packet on the wire, with no file form of its own |
| `rtpdump` | network | Unlicense | yes | 0 | no magic |  |
| `ruby_marshal` | serialization | CC0-1.0 | yes | 1 | 2 bytes | the one instance that unpacks a small integer needs a bitwise complement |
| `saints_row_2_vpp_pc` | game | MIT | yes | 2 | 5 bytes | two instances reading the name tables as their own streams; the entry table is placed |
| `shapefile_index` | geospatial | CC0-1.0 | yes | 0 | no magic |  |
| `shapefile_main` | geospatial | CC0-1.0 | yes | 0 | no magic |  |
| `specpr` | scientific | Unlicense | yes | 2 | no magic | two floating-point instances |
| `ssh_public_key` | security | CC0-1.0 | yes | 0 | no magic |  |
| `sudoers_ts` | log | CC0-1.0 | yes | 0 | no magic |  |
| `tcp_segment` | network | CC0-1.0 | yes | 0 | no magic |  |
| `tls_client_hello` | network | MIT | yes | 0 | no magic |  |
| `tr_dos_image` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `tsm` | database | MIT | yes | 0 | no magic |  |
| `ttf` | font | MIT | yes | 3 | no magic | three instances reading a sub-table through its parent's stream; every table is placed |
| `udp_datagram` | network | CC0-1.0 | yes | 0 | no magic |  |
| `uefi_te` | executable | CC0-1.0 | yes | 1 | no magic | one instance measures a field the IR does not let it measure |
| `uimage` | firmware | CC0-1.0 | yes | 0 | no magic |  |
| `utf8_string` | common | CC0-1.0 | yes | 1 | no magic | the one instance that assembles a code point needs a bitwise or |
| `vfat` | filesystem | CC0-1.0 | yes | 8 | no magic | the packed date and time read most-significant-bit first instead of least, and six instances only zero-pad the parts for display |
| `vlq_base128_be` | common | CC0-1.0 | yes | 0 | no magic |  |
| `vlq_base128_le` | common | CC0-1.0 | yes | 0 | no magic |  |
| `vmware_vmdk` | filesystem | CC0-1.0 | yes | 0 | no magic |  |
| `vp8_duck_ivf` | media | CC0-1.0 | yes | 0 | 8 bytes |  |
| `warcraft_2_pud` | game | CC0-1.0 | yes | 0 | no magic |  |
| `windows_evt_log` | log | CC0-1.0 | yes | 0 | no magic |  |
| `windows_minidump` | windows | CC0-1.0 | yes | 0 | 6 bytes |  |
| `windows_shell_items` | windows | CC0-1.0 | yes | 0 | no magic |  |
| `windows_systemtime` | windows | CC0-1.0 | yes | 0 | no magic |  |
| `wmf` | image | CC0-1.0 | yes | 0 | no magic |  |
| `xwd` | image | CC0-1.0 | yes | 0 | no magic |  |
| `zisofs` | archive | CC0-1.0 | yes | 1 | no magic | one instance with no `pos`, which has no place in the file the IR can name |

## Not bundled: the conversion does not reach the file

Candidates from `HANDOVER-kaitai-gaplist.md` whose report has gaps on the path
through the file: the root's own `seq`, or the type most of the file is made
of. A format that reads the front of a file and then loses the rest is worse
than no format at all, so these wait for the converter to grow. They are not
copied into this repository.

| Format | Licence | Why not |
| --- | --- | --- |
| `android_sparse` | CC0-1.0 | the size of a chunk's header reads `_root.header`, which a field of the same name hides, so no chunk body is placed |
| `dbf` | CC0-1.0 | a record's field widths come from `.length` over the header's field list, which the IR cannot take |
| `gettext_mo` | BSD-2-Clause | the whole file's endianness is chosen from its first word, and the IR has no form for that |
| `nitf` | MIT | every sub-header's count is text read as a number, so twenty-one fields on the path through the file are left as bytes |
| `pcap` | CC0-1.0 | the whole file's endianness is chosen from its magic, and the IR has no form for that |
| `windows_resource_file` | CC0-1.0 | a resource's name is a `repeat-until` over `_`, which the IR cannot say, so everything after the name misplaces |

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

* 21 of the 112 files carry no `meta/title`. Nothing invents one: the
  chooser shows those by their id.
* 22 formats declare a magic and 20 of them sniff. None was dropped for
  having a one-byte magic, because none has one; 2 were dropped for sharing
  a magic with each other, which the table names.
* `ruby_marshal` (`04 08`) and `psx_tim` (`10 00 00 00`) pass the two-byte rule
  with weak evidence: both are version numbers rather than a name. They sniff
  only for a file no builtin claims, which is the whole reason that is
  tolerable.
