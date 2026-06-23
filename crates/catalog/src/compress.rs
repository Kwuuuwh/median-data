use std::path::Path;

/// Compress a file to a `.zst` artifact with zstd at `level` (pipeline-side encode only).
pub fn compress_file(input: &Path, out: &Path, level: i32) -> anyhow::Result<()> {
    let bytes = std::fs::read(input)?;
    let compressed = zstd::stream::encode_all(bytes.as_slice(), level)?;
    std::fs::write(out, compressed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    #[test]
    fn zstd_encode_ruzstd_decode_roundtrip() {
        let original: Vec<u8> = b"SQLite format 3\0catalog-bytes-".repeat(64);
        let compressed = zstd::stream::encode_all(original.as_slice(), 19).unwrap();
        assert_ne!(compressed, original);

        let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed.as_slice()).unwrap();
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, original);
    }
}
