# Kaitai formats against what Qubero parses

Made 2026-09-13 from `meta` of the 185 `.ksy` under
`D:\github\kaitai_struct\formats` (regenerate the raw table with
`python tools/ksy_meta.py D:/github/kaitai_struct/formats`) joined by hand
against `BUILTIN` in `crates/core/src/formats/mod.rs` and `assimp::NAMES`.
Only 40 of the 185 open with a `contents` field at offset 0, so most bundled
formats will be chosen by name rather than sniffed.

## Already parsed by Qubero (do not bundle; useful as converter test pairs)

gltf_binary (assimp glb), gzip, lzh (lha), rar, rpm, xar, zip, sqlite3,
dos_mz, elf, microsoft_pe, mach_o, mach_o_fat (check `macho` covers fat),
swf, iso9660, apple_single_double, zx_spectrum_tap, ines (nes), doom_wad,
quake_pak (check `pak`), dtb, bmp, gif, ico, jpeg, pcx, png, tga, exif
(tiff), glibc_utmp and aix_utmp (utmp), systemd_journal, au, blender_blend
(assimp), id3v1_1 / id3v2_3 / id3v2_4 (id3), ogg, quicktime_mov (mp4),
standard_midi_file, stl (assimp), wav, fasttracker_xm_module, s3m,
python_pickle, windows_lnk_file, cpio_old_le, quake2_md2 and quake_mdl
(check assimp), microsoft_cfb (check whether `thumbsdb` reads CFB generally
or only its own streams).

## Excluded by licence

vdi (GPL-3.0), nt_mdt (GPL-3.0), broadcom_trx (GPL-2.0), lvm2 (GFDL-1.3),
pif (LGPL-2.1), renderware_binary_stream (no licence tag).

## Bundle: absent, permissive, worth having

Common (imports, all CC0/MIT; bundle all six): bcd, bytes_with_io,
dos_datetime, riff, utf8_string, vlq_base128_be, vlq_base128_le.

Archive and firmware: android_img, android_sparse, android_bootldr_qcom,
android_bootldr_asus, android_bootldr_huawei, android_dto, android_super,
mozilla_mar, uimage, uefi_te, phar_without_stub, zisofs.

Filesystems and disks: mbr_partition_table, gpt_partition_table,
apm_partition_table, amlogic_emmc_partitions, vfat, ext2, cramfs, luks,
vmware_vmdk, btrfs_stream, tr_dos_image.

Executables: dex, java_class, python_pyc_27, android_nanoapp_header.

Databases and serialisation: dbf, gettext_mo, tsm, bson, msgpack,
asn1_der, google_protobuf, ruby_marshal, php_serialized_value, chrome_pak.

Images and fonts: dicom, nitf, wmf, xwd, icc_4, gimp_brush, psx_tim,
pcx_dcx, ttf, pcf_font, grub2_font.

Media: avi, creative_voice_file, magicavoxel_vox, vp8_duck_ivf,
genmidi_op2, mac_os_resource_snd.

Windows and macOS: regf, windows_minidump, windows_evt_log,
windows_resource_file, windows_shell_items, windows_systemtime, ds_store,
resource_fork, compressed_resource, dcmp_0, dcmp_1, dcmp_2,
dcmp_variable_length_integer.

Network captures and packets: pcap, microsoft_network_monitor_v2, rtpdump,
packet_ppi, ethernet_frame, ipv4_packet, ipv6_packet, tcp_segment,
udp_datagram, icmp_packet, dns_packet, tls_client_hello, hccap, hccapx,
dime_message, bitcoin_transaction.

Scientific and hardware: avantes_roh60, specpr, nt_mdt_pal, edid,
mifare_classic, shapefile_main, shapefile_index, mcap.

Games: allegro_dat, dune_2_pak, fallout_dat, fallout2_dat, ftl_dat,
gran_turismo_vol, heaps_pak, heroes_of_might_and_magic_agg,
heroes_of_might_and_magic_bmp, minecraft_nbt, saints_row_2_vpp_pc,
warcraft_2_pud.

Logs and security: hashcat_restore, sudoers_ts, efivar_signature_list,
openpgp_message, ssh_public_key.

## Left out on purpose

code_6502 (machine code, `Ty::Insn` territory), monomakh_sapr_chg,
respack, andes_firmware, android_opengl_shaders_cache, websocket and the
five some_ip specs (wire protocols with no file form), rtp_packet and
rtcp_payload (same), protocol_body (a dispatch table, only useful under
ipv4/ipv6, which import it: bundle it if those convert).

The bundling agent decides per format from the conversion report: a format
whose report has gaps in its main path does not ship until the gap is
closed.
