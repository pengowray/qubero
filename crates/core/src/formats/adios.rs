//! ADIOS2 BP files: what the large simulations of fusion, climate and
//! cosmology write their output as. Read from the ADIOS2 sources' serializers
//! (`toolkit/format/bp`, `bp5` and the BP4 and BP5 writers, release 2.12.1),
//! and checked against files that release wrote.
//!
//! Three versions, and only the oldest is one file. **BP3**, the format of
//! ADIOS 1 carried into ADIOS2, is a run of process groups and then a footer
//! that indexes them. **BP4** splits the same records into a directory:
//! `data.0` to `data.N` hold the process groups, `md.0` holds every step's
//! index, and `md.idx` says where in `md.0` each step starts. **BP5** keeps the
//! directory and replaces the indices with FFS, a self-describing binary
//! serialization of its own: `md.idx` again, `md.0` holding each step's FFS
//! records, `mmd.0` holding the FFS formats those records are written in, and
//! `data.N` holding nothing but the bytes the records point at.
//!
//! Qubero opens one file, so every file of a directory is a template of its
//! own. A record in one file that points into another, a step in `md.0` naming
//! a block in `data.0`, is read as the number it is and not followed.
//!
//! **Process groups.** What a writer's rank puts down in one step: the IO's
//! name, whether its arrays are column-major, the step, the transports, and
//! then every variable block it wrote, each a header with the variable's name,
//! type and dimensions and then its values, and last the attributes, each
//! written once in the first group that had it. BP4 brackets the group, each
//! variable and each attribute with four letters, `[PGI` and `PGI]` and the
//! like, which BP3 does not have. The header carries the type and the
//! dimensions, so a data file is read to its values without its index.
//!
//! **Indices.** Three per step: one entry per process group saying where it
//! starts, one per variable and one per attribute. A variable's entry is its
//! name and type and then one characteristic set per block, and a set is a
//! list of tagged records: the step, the file, the dimensions, the minimum and
//! maximum, where the block starts. Which records a set holds, and in what
//! order, is up to the writer, so each is read by its tag and a record whose
//! width depends on another asks for it by tag: an attribute's value is as
//! many numbers as its dimensions record says, and a sub-block minimum table
//! as wide as the dimensions are many.
//!
//! **BP5.** The index file is records of a kind and a length, so a record of a
//! kind this does not know is passed over by its length. A step in `md.0` is
//! a total and then, per writer, the size of its metadata block and of its
//! attribute block, and the writer count is in the index file, not here. So a
//! step is read into its blocks only when one writer's two sizes and the eight
//! bytes each takes add up to the total, which is a check a step written by two
//! writers cannot pass. What is inside a block is an FFS record, see
//! [`ffs::ffs_record`] for how far that reads and what it would take to go further.

mod bp3;
mod bp4;
mod bp5;
mod ffs;
mod groups;
mod indices;
mod recognise;
mod shared;
#[cfg(test)]
mod tests;

pub use bp3::adios_bp3;
pub use bp4::{adios_bp4_data, adios_bp4_index, adios_bp4_metadata};
pub use bp5::{adios_bp5_index, adios_bp5_metadata};
pub use ffs::adios_bp5_metametadata;
pub use recognise::{sniff_agreeing, sniff_signed};
