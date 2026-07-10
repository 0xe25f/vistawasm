//! Stream helpers for future chunked DEM decoding.

/// Incremental decode status for host-managed streams.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
  /// More bytes are required before metadata can be decoded.
  NeedMoreData,
  /// Enough bytes are available to decode metadata.
  MetadataReady,
  /// The stream has completed.
  Complete,
}
