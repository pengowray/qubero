//! What a container holds: HDF5 objects, ELF sections and symbols, ROOT trees, ISO 9660 directories.

use crate::*;

/// One object of an HDF5 file, in the file's own terms.
#[derive(Serialize)]
pub(crate) struct ContentDto {
    /// Where it is in the template, so picking a row moves the cursor.
    pub(crate) path: Vec<usize>,
    /// The path it goes by inside the file: `/obs/n_genes`.
    pub(crate) name: String,
    pub(crate) group: bool,
    /// What the file calls it, where it says: `dataframe`, `csr_matrix`.
    pub(crate) encoding: String,
    pub(crate) shape: Vec<f64>,
    /// What one element is.
    pub(crate) element: String,
    /// Which of the three ways its bytes are kept: "contiguous", "compact",
    /// "chunked", or nothing at all for a group.
    pub(crate) storage: &'static str,
    /// How many bytes, where they are in one run.
    pub(crate) bytes: f64,
    /// The chunk it is kept in, where it is kept in chunks.
    pub(crate) chunk_dims: Vec<f64>,
    /// The filters its chunks were written through, in that order.
    pub(crate) filters: Vec<String>,
    /// Where its object header is.
    pub(crate) address: f64,
}

/// What an HDF5 file holds, and what kind of file it is.
#[derive(Serialize)]
pub(crate) struct ContentsDto {
    pub(crate) objects: Vec<ContentDto>,
    pub(crate) total: f64,
    /// Whether it is an AnnData object, and what the root group calls itself.
    pub(crate) anndata: bool,
    pub(crate) encoding: String,
    pub(crate) rows: f64,
    pub(crate) columns: f64,
}

/// One node of an HDF5 B-tree, as much of it as a picture of the tree needs.
#[derive(Serialize)]
pub(crate) struct TreeNodeDto {
    /// Where the node is in the template. Empty for a version 2 node below the
    /// root, which the template does not place: the host takes such a box to
    /// its bytes and does not try to open it in the Listing, because there is
    /// no field there to open.
    pub(crate) path: Vec<usize>,
    /// Index into the node list, or -1 for the root.
    pub(crate) parent: f64,
    /// `index` for a `TREE` or a `BTIN`, `links` for the symbol table node a
    /// version 1 group tree hangs under its bottom row, `leaf` for a `BTLF`.
    /// The words are the host's to translate; what crosses is which of the
    /// three kinds of node this is.
    pub(crate) kind: &'static str,
    /// The four bytes written at `address`. Sent rather than worked out from
    /// `kind` and the tree's version, so that what a reader is told to expect
    /// at an address is what the walk checked for there.
    pub(crate) sign: &'static str,
    pub(crate) address: f64,
    pub(crate) size_bits: f64,
    /// What the file wrote as this node's level, for a version 1 index node.
    /// Zero for a link table, which sits below the levels rather than on one.
    /// A version 2 node writes no level, so this is the header's depth less
    /// the rows walked to reach it, which keeps level 0 meaning the bottom row
    /// of index nodes in both versions.
    pub(crate) level: f64,
    /// Rows below the root, counted by the walk.
    pub(crate) depth: f64,
    /// `entries_used`, `symbol_count`, or a version 2 node's record count. The
    /// file's own number and no fraction of anything. A version 2 internal
    /// node points at one more child than it holds records, because a record
    /// sits between every two children.
    pub(crate) entries: f64,
    /// The ends of the node's key range, or empty where the walk could not
    /// settle both, and empty throughout a version 2 group tree, whose records
    /// hold the hash of a name rather than the name. For a version 1 group
    /// tree these are link names; for a chunk tree of either version they are
    /// the comma-separated numbers of a chunk's offset in the dataset.
    pub(crate) first_key: String,
    pub(crate) last_key: String,
    /// True when children of this node were not reached, so its count stands
    /// and its range does not.
    pub(crate) truncated: bool,
    /// Where the node's first entry starts, in bits from `address`, and how
    /// many bits one entry takes, so the host can place all `entries` of them
    /// without a list: entry `i` is at `address * 8 + first_entry_bits + i *
    /// entry_bits`. What one entry covers is a key and a child address for a
    /// `TREE`, one symbol table entry for an `SNOD`, one record for a `BTLF`
    /// or a `BTIN`. A `TREE`'s closing key and a `BTIN`'s child pointers are
    /// not entries and are not on the stride.
    ///
    /// Both zero where the walk could not settle the stride, which is the one
    /// case a host must leave the node undivided rather than take the zero for
    /// a width.
    pub(crate) first_entry_bits: f64,
    pub(crate) entry_bits: f64,
}

/// One HDF5 B-tree, walked into the shape it has in the file.
#[derive(Serialize)]
pub(crate) struct TreeDto {
    /// `group` for a tree indexing a group's links, `chunk` for one indexing a
    /// dataset's chunks, `other` for a version 2 tree indexing neither: a
    /// file's shared messages, an object's attributes, or its huge objects.
    /// Only a version 1 group tree has link tables under its bottom row, so
    /// `job` alone does not settle the silhouette; `version` does.
    pub(crate) job: &'static str,
    /// 1 or 2: which of the two structures this is.
    pub(crate) version: f64,
    /// The record type byte a version 2 header writes, and the name the
    /// specification gives it. Zero and empty for a version 1 tree, which has
    /// no such byte.
    pub(crate) record_type: f64,
    pub(crate) record_type_name: &'static str,
    /// How far the walk got with the records: `read` where what a record holds
    /// is known here, `unread` where the specification names the type and this
    /// does not read it, `unknown` where no version of the specification names
    /// it. The shape is drawn whichever of the three it is, because the shape
    /// depends only on the record size, and the host says outright when the
    /// records behind a drawn shape were not read.
    pub(crate) records: &'static str,
    /// How many records the tree holds in all, which a version 2 header writes
    /// in a field of its own. Zero for a version 1 tree. Not the sum of the
    /// nodes' counts: a version 2 internal node holds records the leaves under
    /// it do not repeat, so a host that summed the bottom row would print a
    /// total the file disagrees with, and this stands when the walk was capped.
    pub(crate) records_total: f64,
    pub(crate) nodes: Vec<TreeNodeDto>,
    pub(crate) omitted: f64,
    /// How many numbers one chunk key holds. Zero for a group tree and for a
    /// tree whose records were not read.
    pub(crate) coords: f64,
    /// True where the last of those numbers is the always-zero offset within
    /// an element, which a version 1 chunk key ends with and a version 2
    /// record does not. Without it a reader counting the numbers in
    /// `950, 950, 0` gets a rank one too high.
    pub(crate) coords_pad: bool,
}

/// The named parts of an ELF file. Unlike the storage template, these have
/// resolved section and symbol names rather than string-table offsets.
#[derive(Serialize)]
pub(crate) struct ElfContentsDto {
    pub(crate) sections: Vec<ElfSectionDto>,
    pub(crate) symbols: Vec<ElfSymbolDto>,
    pub(crate) symbol_total: f64,
}

/// What a ROOT file holds, read beside the template.
#[derive(Serialize)]
pub(crate) struct RootContentsDto {
    pub(crate) classes: Vec<RootClassDto>,
    /// Where the `StreamerInfo` record is in the template.
    pub(crate) schema_path: Vec<usize>,
    pub(crate) trees: Vec<RootTreeDto>,
    pub(crate) tree_total: f64,
    /// What stopped the walk, empty where nothing did.
    pub(crate) trouble: String,
}

#[derive(Serialize)]
pub(crate) struct RootClassDto {
    pub(crate) name: String,
    pub(crate) version: f64,
    pub(crate) checksum: f64,
    pub(crate) members: Vec<RootMemberDto>,
}

#[derive(Serialize)]
pub(crate) struct RootMemberDto {
    pub(crate) name: String,
    /// The C++ type as the file spells it.
    pub(crate) type_name: String,
    /// ROOT's own type code, which says how the member is written.
    pub(crate) code: f64,
    pub(crate) size: f64,
    /// The dimensions of a fixed array, empty for a single value.
    pub(crate) dims: Vec<f64>,
    /// True where this is a base class rather than a member of its own.
    pub(crate) base: bool,
    pub(crate) comment: String,
}

#[derive(Serialize)]
pub(crate) struct RootTreeDto {
    /// Where the tree's key is in the template, so picking a row moves the
    /// cursor.
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) entries: f64,
    pub(crate) address: f64,
    pub(crate) branches: Vec<RootBranchDto>,
    pub(crate) branch_total: f64,
    pub(crate) trouble: String,
}

#[derive(Serialize)]
pub(crate) struct RootBranchDto {
    pub(crate) name: String,
    pub(crate) title: String,
    /// `TBranch` for numbers, `TBranchElement` for a member of a split C++
    /// class.
    pub(crate) class: String,
    pub(crate) depth: f64,
    pub(crate) entries: f64,
    pub(crate) total_bytes: f64,
    pub(crate) zip_bytes: f64,
    pub(crate) basket_total: f64,
    /// One value's width in bytes and how many of them an entry holds, or zero
    /// for a branch whose values are not read.
    pub(crate) width: f64,
    pub(crate) per_entry: f64,
    pub(crate) floating: bool,
    pub(crate) unsigned: bool,
    /// Why the values are not read, empty where they are.
    pub(crate) unread: String,
    pub(crate) leaves: Vec<RootLeafDto>,
    pub(crate) baskets: Vec<RootBasketDto>,
}

#[derive(Serialize)]
pub(crate) struct RootLeafDto {
    pub(crate) name: String,
    pub(crate) class: String,
    pub(crate) len: f64,
    pub(crate) width: f64,
    pub(crate) unsigned: bool,
    /// The leaf that counts this one, empty where the count is fixed.
    pub(crate) counted_by: String,
}

/// One basket: a record of the file that no key list lists and no field
/// places, so the host takes a row to its bytes and cannot open it in the
/// Listing.
#[derive(Serialize)]
pub(crate) struct RootBasketDto {
    pub(crate) address: f64,
    pub(crate) bytes: f64,
    pub(crate) first_entry: f64,
    pub(crate) entries: f64,
}

#[derive(Serialize)]
pub(crate) struct ElfSectionDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    pub(crate) kind: f64,
    pub(crate) address: f64,
    pub(crate) offset: f64,
    pub(crate) size: f64,
}

#[derive(Serialize)]
pub(crate) struct ElfSymbolDto {
    pub(crate) path: Vec<usize>,
    pub(crate) source_bits: f64,
    pub(crate) name: String,
    pub(crate) kind: f64,
    pub(crate) section: f64,
    pub(crate) value: f64,
    pub(crate) size: f64,
}

#[derive(Serialize)]
pub(crate) struct IsoVolumeDto {
    pub(crate) descriptor_path: Vec<usize>,
    pub(crate) volume: String,
    pub(crate) joliet: bool,
    pub(crate) block_size: f64,
    pub(crate) blocks: f64,
    pub(crate) root_extent: f64,
    pub(crate) root_size: f64,
    pub(crate) root_source_bits: f64,
}

#[derive(Serialize)]
pub(crate) struct IsoDirectoryDto {
    pub(crate) entries: Vec<IsoEntryDto>,
    pub(crate) total: f64,
}

#[derive(Serialize)]
pub(crate) struct IsoEntryDto {
    pub(crate) name: String,
    pub(crate) directory: bool,
    pub(crate) extent: f64,
    pub(crate) size: f64,
    pub(crate) source_bits: f64,
    pub(crate) extents: f64,
    pub(crate) multi_extent: bool,
}
