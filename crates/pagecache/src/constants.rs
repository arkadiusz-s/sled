use super::*;

/// Indicates that the following buffer corresponds
/// to a reservation for an in-memory operation that
/// failed to complete. It should be skipped during
/// recovery.
pub const FAILED_FLUSH: u8 = 0;

/// Indicates that the following buffer contains
/// valid data, stored inline.
pub const INLINE_FLUSH: u8 = 1;

/// Indicates that the following buffer contains
/// valid data, stored blobly.
pub const BLOB_FLUSH: u8 = 2;

/// Indicates that the following buffer is used
/// as padding to fill out the rest of the segment
/// before sealing it.
pub const SEGMENT_PAD: u8 = 3;

/// The EVIL_BYTE is written as a canary to help
/// detect torn writes.
pub const EVIL_BYTE: u8 = 6;

/// Log messages have a header of this length.
pub const MSG_HEADER_LEN: usize = 15;

/// Log segments have a header of this length.
pub const SEG_HEADER_LEN: usize = 10;

/// Log segments have a trailer of this length.
pub const SEG_TRAILER_LEN: usize = 10;

/// Log messages that are stored as external blobs
/// contain a value (in addition to their header)
/// of this length.
pub const BLOB_INLINE_LEN: usize = std::mem::size_of::<Lsn>();
